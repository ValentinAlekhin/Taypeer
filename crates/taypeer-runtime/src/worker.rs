//! Child-side dispatch. Only this process constructs a plaintext database service.
#[cfg(any(test, feature = "ui-test-support"))]
mod faults;
mod input;
#[cfg(test)]
mod tests;

use crate::{
    Command, RuntimeError,
    cipher_ipc::{Channel, RemotePersistence},
    protocol::Boot,
};
use serde::Serialize;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};
use taypeer_services::{DatabaseService, SessionToken};
use zeroize::Zeroize;

/// Serve one database over inherited private pipes until lock or EOF.
/// Never attach this entry point to a public socket or ordinary terminal output.
pub fn run_worker(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
) -> Result<(), RuntimeError> {
    run_with_open(reader, writer, open_native)
}

type LoadProfile =
    fn(&std::path::Path) -> Result<crate::profile::NativeProfile, crate::profile::ProfileError>;

/// Serve synthetic UI fixtures with explicitly selected file credentials.
#[cfg(feature = "ui-test-support")]
pub fn run_test_worker(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
) -> Result<(), RuntimeError> {
    run_with_profile(
        reader,
        writer,
        |boot, channel, service| {
            open_profile(
                boot,
                channel,
                service,
                crate::profile::NativeProfile::load_test,
                |port| Box::new(faults::Persistence(port)),
            )
        },
        crate::profile::NativeProfile::load_test,
    )
}

fn run_with_open(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    open: impl FnOnce(
        &mut Boot,
        &Arc<Mutex<Channel>>,
        &mut DatabaseService,
    ) -> Result<SessionToken, RuntimeError>,
) -> Result<(), RuntimeError> {
    run_with_profile(reader, writer, open, crate::profile::NativeProfile::load)
}

fn run_with_profile(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    open: impl FnOnce(
        &mut Boot,
        &Arc<Mutex<Channel>>,
        &mut DatabaseService,
    ) -> Result<SessionToken, RuntimeError>,
    load: LoadProfile,
) -> Result<(), RuntimeError> {
    let (reader, _lifetime) = input::Incoming::start(reader);
    let channel = Arc::new(Mutex::new(Channel::new(reader, writer)));
    let mut boot: Boot = channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .read()?;
    if let Some(invitation) = boot.invitation.take() {
        let result = (|| {
            let profile = load(&boot.profile)?;
            let (author, transport) = if profile.is_linux_lazy() {
                let (author, capability) =
                    profile.prepare_join_credentials(&invitation, boot.password.as_bytes())?;
                let transport =
                    taypeer_trust::TransportKey::from_seed(&capability.transport_seed).public();
                let reply = channel
                    .lock()
                    .map_err(|_| RuntimeError::Transport)?
                    .io(crate::cipher_ipc::IoRequest::Activate(Box::new(capability)))?;
                if !matches!(reply, crate::cipher_ipc::IoValue::Done) {
                    return Err(RuntimeError::Protocol);
                }
                (author, transport)
            } else {
                (profile.author()?, profile.transport_public())
            };
            let identity = taypeer_trust::Identity::new(author.public(), transport)
                .map_err(|_| RuntimeError::Protocol)?;
            let proof = taypeer_trust::JoinProof::sign(&invitation, identity, &author)
                .map_err(|_| RuntimeError::Protocol)?;
            value(&proof)
        })();
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(result)?;
        // Retain staging authentication solely in this process until receipt is verified.
        loop {
            let command = channel
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .read::<Command>()?;
            match command {
                Command::Lock => {
                    boot.password.zeroize();
                    channel
                        .lock()
                        .map_err(|_| RuntimeError::Transport)?
                        .response(Ok(Value::Null))?;
                    return Ok(());
                }
                Command::BindInvitation { path, password } => {
                    let bound = (|| {
                        let snapshot = taypeer_storage::ArchiveSnapshot::open(&path, None)
                            .map_err(crate::cipher_ipc::storage)?;
                        let key = DatabaseService::authenticate_archive_credentials(
                            &snapshot, &password,
                        )?;
                        let profile = load(&boot.profile)?;
                        if profile.is_linux_lazy() {
                            profile.bind_join_credentials(
                                &snapshot,
                                boot.password.as_bytes(),
                                &key,
                            )?;
                            let history = DatabaseService::authenticate_archive_keyring(
                                &snapshot, &password,
                            )?;
                            let _verified = profile.database_credentials(
                                &snapshot.chain().head().database,
                                snapshot
                                    .chain()
                                    .root()
                                    .map_err(|_| RuntimeError::Protocol)?,
                                snapshot.chain().head().epoch,
                                &key,
                                &history,
                            )?;
                            profile.finalize_authenticated_credentials(
                                &snapshot.chain().head().database,
                                snapshot
                                    .chain()
                                    .root()
                                    .map_err(|_| RuntimeError::Protocol)?,
                                &key,
                                snapshot.chain().head().epoch,
                            )?;
                        }
                        Ok(Value::Null)
                    })();
                    channel
                        .lock()
                        .map_err(|_| RuntimeError::Transport)?
                        .response(bound)?;
                }
                _ => return Err(RuntimeError::Protocol),
            }
        }
    }
    let mut service = DatabaseService::new();
    let opened = open(&mut boot, &channel, &mut service);
    boot.password.zeroize();
    let session = match opened {
        Ok(session) => session,
        Err(error) => {
            channel
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .response(Err(error))?;
            return Ok(());
        }
    };
    if service.can_write(&session)?
        && let Err(error) = service.apply_received(&session)
    {
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(Err(error.into()))?;
        return Ok(());
    }
    channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .response(value(&session.database))?;
    let outcome = serve(&channel, &mut service, &session, &boot, load);
    // EOF and protocol errors also revoke access. A broken parent cannot receive
    // a draft failure, but the process must still stop holding the document.
    let closed = service.lock_all_checked().map_err(RuntimeError::from);
    outcome.and(closed)
}

fn open_native(
    boot: &mut Boot,
    channel: &Arc<Mutex<Channel>>,
    service: &mut DatabaseService,
) -> Result<SessionToken, RuntimeError> {
    open_profile(
        boot,
        channel,
        service,
        crate::profile::NativeProfile::load,
        |port| Box::new(port),
    )
}
fn open_profile(
    boot: &mut Boot,
    channel: &Arc<Mutex<Channel>>,
    service: &mut DatabaseService,
    load: LoadProfile,
    wrap: fn(RemotePersistence) -> Box<dyn taypeer_storage::CipherPersistence>,
) -> Result<SessionToken, RuntimeError> {
    let profile = load(&boot.profile)?;
    let form = boot.create_form.take().or_else(|| {
        boot.create_name
            .take()
            .map(|name| taypeer_services::CreateDatabase {
                name,
                description: None,
                policy: Default::default(),
            })
    });
    if profile.is_linux_lazy() {
        return open_linux(boot, channel, service, profile, form, wrap);
    }
    let seed = match form {
        Some(form) => {
            let author = profile.author()?;
            let identity =
                taypeer_trust::Identity::new(author.public(), profile.transport_public())
                    .map_err(|_| RuntimeError::Protocol)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| RuntimeError::Protocol)?
                .as_millis();
            Some(DatabaseService::prepare_managed_form(
                form,
                boot.password.as_bytes(),
                &author,
                identity,
                now.try_into().map_err(|_| RuntimeError::Protocol)?,
            )?)
        }
        None => None,
    };
    let port = RemotePersistence::attach(
        Arc::clone(channel),
        boot.path.clone(),
        boot.spool.clone(),
        seed,
    )?;
    service
        .open_managed(wrap(port), boot.password.as_bytes(), || {
            profile
                .author()
                .map(Some)
                .map_err(|_| taypeer_services::ServiceError::Credentials)
        })
        .map_err(RuntimeError::from)
}

fn open_linux(
    boot: &mut Boot,
    channel: &Arc<Mutex<Channel>>,
    service: &mut DatabaseService,
    profile: crate::profile::NativeProfile,
    form: Option<taypeer_services::CreateDatabase>,
    wrap: fn(RemotePersistence) -> Box<dyn taypeer_storage::CipherPersistence>,
) -> Result<SessionToken, RuntimeError> {
    use crate::cipher_ipc::{IoRequest, IoValue};
    use taypeer_storage::ArchiveSnapshot;
    use taypeer_trust::{AuthorKey, Identity, TransportKey};
    let (seed, author, capability, authenticated, credential_key) = if let Some(form) = form {
        let author = AuthorKey::generate().map_err(|_| RuntimeError::Protocol)?;
        let transport = TransportKey::generate().map_err(|_| RuntimeError::Protocol)?;
        let identity = Identity::new(author.public(), transport.public())
            .map_err(|_| RuntimeError::Protocol)?;
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_err(|_| RuntimeError::Protocol)?
            .as_millis();
        let seed = DatabaseService::prepare_managed_form(
            form,
            boot.password.as_bytes(),
            &author,
            identity,
            now.try_into().map_err(|_| RuntimeError::Protocol)?,
        )?;
        let head = seed.controls.last().ok_or(RuntimeError::Protocol)?;
        let root = seed
            .controls
            .first()
            .ok_or(RuntimeError::Protocol)?
            .hash()
            .map_err(|_| RuntimeError::Protocol)?;
        let checkpoint = seed
            .objects
            .iter()
            .find(|object| object.descriptor().digest == seed.checkpoint)
            .ok_or(RuntimeError::Protocol)?;
        let key = checkpoint
            .unlock_key(boot.password.as_bytes())
            .map_err(crate::cipher_ipc::storage)?;
        let capability = profile.create_database_credentials(
            &head.body.database,
            root,
            head.body.epoch,
            &key,
            &author,
            &transport,
        )?;
        (Some(seed), Some(author), capability, None, Some(key))
    } else {
        // Authentication and local-key acquisition occur in this worker, before host activation.
        let snapshot =
            ArchiveSnapshot::open(&boot.path, None).map_err(crate::cipher_ipc::storage)?;
        let key =
            DatabaseService::authenticate_archive_credentials(&snapshot, boot.password.as_bytes())?;
        profile.bind_join_credentials(&snapshot, boot.password.as_bytes(), &key)?;
        let history =
            DatabaseService::authenticate_archive_keyring(&snapshot, boot.password.as_bytes())?;
        let (author, capability) = profile.database_credentials(
            &snapshot.chain().head().database,
            snapshot
                .chain()
                .root()
                .map_err(|_| RuntimeError::Protocol)?,
            snapshot.chain().head().epoch,
            &key,
            &history,
        )?;
        (
            None,
            author,
            capability,
            Some(snapshot.fingerprint()),
            Some(key),
        )
    };
    let persistent = capability.local_state_seed.is_some();
    let epoch = seed
        .as_ref()
        .and_then(|seed| seed.controls.last().map(|control| control.body.epoch));
    let reply = channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .io(IoRequest::Activate(Box::new(capability)))?;
    if !matches!(reply, IoValue::Done) {
        return Err(RuntimeError::Protocol);
    }
    let port = RemotePersistence::attach(
        Arc::clone(channel),
        boot.path.clone(),
        boot.spool.clone(),
        seed,
    )?;
    if let Some(expected) = authenticated
        && taypeer_storage::CipherPersistence::snapshot(&port)
            .map_err(crate::cipher_ipc::storage)?
            .fingerprint()
            != expected
    {
        return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Changed));
    }
    if persistent {
        let current = taypeer_storage::CipherPersistence::snapshot(&port)
            .map_err(crate::cipher_ipc::storage)?;
        let epoch = epoch.unwrap_or(current.chain().head().epoch);
        profile.finalize_authenticated_credentials(
            &current.chain().head().database,
            current.chain().root().map_err(|_| RuntimeError::Protocol)?,
            credential_key.as_ref().ok_or(RuntimeError::Protocol)?,
            epoch,
        )?;
        let reply = channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .io(IoRequest::FinalizeCredentials { epoch })?;
        if !matches!(reply, IoValue::Done) {
            return Err(RuntimeError::Protocol);
        }
    }
    service
        .open_managed(wrap(port), boot.password.as_bytes(), || Ok(author))
        .map_err(RuntimeError::from)
}

fn serve(
    channel: &Arc<Mutex<Channel>>,
    service: &mut DatabaseService,
    session: &SessionToken,
    boot: &Boot,
    load: LoadProfile,
) -> Result<(), RuntimeError> {
    loop {
        let command: Command = channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .read()?;
        let lock = matches!(command, Command::Lock);
        let credential_rotation = matches!(&command, Command::RotatePassword { .. })
            && load(&boot.profile)?.is_linux_lazy();
        let result = match command {
            Command::RecoverTrust {
                path,
                operation,
                password,
            } => recover(
                service,
                session,
                channel,
                boot,
                Recovery {
                    path: &path,
                    operation,
                    password: &password,
                },
                load,
            ),
            Command::RotatePassword {
                operation,
                password,
                revoke,
            } if cfg!(target_os = "linux") => {
                let profile = load(&boot.profile)?;
                if profile.is_linux_lazy() {
                    service
                        .rotate_password_with_credentials(
                            session,
                            operation,
                            &password,
                            revoke,
                            |stage, old, new, epoch| {
                                use crate::cipher_ipc::{IoRequest, IoValue};
                                use taypeer_services::{EpochCredentialStage, ServiceError};
                                let request = match stage {
                                    EpochCredentialStage::Prepare => {
                                        profile.prepare_credential_rotation(
                                            &session.database,
                                            old,
                                            new,
                                            epoch,
                                        )?;
                                        IoRequest::PrepareCredentials { epoch }
                                    }
                                    EpochCredentialStage::Finalize => {
                                        profile.finalize_credential_rotation(&session.database)?;
                                        IoRequest::FinalizeCredentials { epoch }
                                    }
                                };
                                let reply = channel
                                    .lock()
                                    .map_err(|_| ServiceError::Credentials)?
                                    .io(request)
                                    .map_err(|error| match error {
                                        RuntimeError::Service(error) => error,
                                        _ => ServiceError::Credentials,
                                    })?;
                                if !matches!(reply, IoValue::Done) {
                                    return Err(ServiceError::Credentials);
                                }
                                Ok(())
                            },
                        )
                        .map(|()| Value::Null)
                        .map_err(RuntimeError::from)
                } else {
                    dispatch(
                        service,
                        session,
                        Command::RotatePassword {
                            operation,
                            password,
                            revoke,
                        },
                    )
                }
            }
            command => dispatch(service, session, command),
        };
        let uncertain = credential_rotation
            && matches!(
                result,
                Err(RuntimeError::Service(
                    taypeer_services::ServiceError::Storage(
                        taypeer_storage::Error::CommitUncertain
                    )
                ))
            );
        if uncertain {
            // The database may already have advanced; no subsequent edit may use this generation.
            let _closed = service.lock_all_checked();
        }
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(result)?;
        if lock || uncertain {
            return Ok(());
        }
    }
}

struct Recovery<'a> {
    path: &'a std::path::Path,
    operation: taypeer_trust::Digest,
    password: &'a [u8],
}
fn recover(
    service: &DatabaseService,
    session: &SessionToken,
    channel: &Arc<Mutex<Channel>>,
    boot: &Boot,
    request: Recovery<'_>,
    load: LoadProfile,
) -> Result<Value, RuntimeError> {
    let Recovery {
        path,
        operation,
        password,
    } = request;
    use crate::cipher_ipc::{IoRequest, IoValue, Seed, spool_objects};
    let profile = load(&boot.profile)?;
    let author = profile.author()?;
    let identity = taypeer_trust::Identity::new(author.public(), profile.transport_public())
        .map_err(|_| RuntimeError::Protocol)?;
    if path.try_exists().map_err(|_| RuntimeError::Transport)? {
        let root = service.trust_recovery_retry(session, path, password, &identity, operation)?;
        return Ok(json!({"root": root, "database": session.database, "path": path}));
    }
    let seed = service.prepare_trust_recovery(session, password, identity, operation)?;
    let root = seed
        .controls
        .first()
        .ok_or(RuntimeError::Protocol)?
        .hash()
        .map_err(|_| RuntimeError::Protocol)?;
    let mut spools = Vec::new();
    let objects = spool_objects(seed.objects, &boot.spool, &mut spools)?;
    let reply = channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .io(IoRequest::Recover {
            path: path.to_owned(),
            seed: Seed {
                controls: seed.controls,
                objects,
                checkpoint: seed.checkpoint,
                baseline: seed.baseline,
            },
        })?;
    if !matches!(reply, IoValue::Done) {
        return Err(RuntimeError::Protocol);
    }
    Ok(json!({"root": root, "database": session.database, "path": path}))
}

fn value(data: &impl Serialize) -> Result<Value, RuntimeError> {
    serde_json::to_value(data).map_err(|_| RuntimeError::Protocol)
}

fn dispatch(
    service: &mut DatabaseService,
    session: &SessionToken,
    command: Command,
) -> Result<Value, RuntimeError> {
    Ok(match command {
        Command::RecoverTrust { .. } => return Err(RuntimeError::Protocol),
        Command::ApplyReceived => value(&service.apply_received(session)?.value)?,
        Command::CollectReceived => value(&service.collect_received(session)?.value)?,
        Command::ReceivedSources => value(&service.received_sources(session)?.value)?,
        Command::InspectReceived(change) => {
            value(&service.inspect_received(session, &change)?.value)?
        }
        Command::RevealReceived { change, entry } => value(
            &service
                .reveal_received(session, &change, &entry)?
                .value
                .expose(),
        )?,
        Command::DiscardReceived(change) => value(&service.discard_received(session, &change)?)?,
        Command::ExtractReceived {
            change,
            entry,
            group,
            operation,
        } => value(
            &service
                .extract_received_in(session, &change, &entry, group, &operation)?
                .value,
        )?,
        Command::Authority => value(&service.authority(session)?)?,
        Command::SessionAuthority => value(&service.session_authority(session)?)?,
        Command::Compatibility => value(&service.compatibility(session)?)?,
        Command::CreateInvitation => {
            let (invitation, secret) = service.create_invitation(session, unix_seconds()?)?;
            value(&(invitation, secret.expose()))?
        }
        Command::ApproveInvitation(request) => {
            value(&service.approve_invitation(session, request, unix_seconds()?)?)?
        }
        Command::CloseInvitation { request, reject } => {
            service.close_invitation(session, request, reject)?;
            Value::Null
        }
        Command::ConsentManagement(operation) => {
            value(&service.consent_management(session, operation)?)?
        }
        Command::TransferManagement(consent) => {
            service.transfer_management(session, consent)?;
            Value::Null
        }
        Command::RotatePassword {
            operation,
            password,
            revoke,
        } => {
            service.rotate_password(session, operation, &password, revoke)?;
            Value::Null
        }
        Command::DatabasePolicy => value(&service.database_policy(session)?.value)?,
        Command::SetDatabasePolicy {
            operation,
            policy,
            password,
        } => {
            service.set_database_policy(
                session,
                operation,
                policy,
                password.as_ref().map(|p| p.as_slice()),
            )?;
            Value::Null
        }
        Command::IconPreview(target) => value(&service.icon_preview(session, &target)?)?,
        Command::BinaryView(target) => value(&service.binary_view(session, &target)?.value)?,
        Command::EditBinary { request, operation } => {
            value(&service.edit_binary(session, &request, &operation)?.value)?
        }
        Command::ExportBinary {
            target,
            blob,
            path,
            overwrite,
        } => value(
            &service
                .export_binary(session, &target, &blob, &path, overwrite)?
                .value,
        )?,
        Command::StorageUsage => value(&service.storage_usage(session)?.value)?,
        Command::CollectBlobs(operation) => {
            value(&service.collect_blobs(session, &operation)?.value)?
        }
        Command::GroupFavicons {
            group,
            recursive,
            replace,
            operation,
        } => value(
            &service
                .group_favicons(session, &group, recursive, replace, &operation)?
                .value,
        )?,
        Command::Tree => value(&service.tree(session)?.value)?,
        Command::Trash => value(&service.trash(session)?.value)?,
        Command::Inspect(target) => value(&service.inspect_object(session, &target)?.value)?,
        Command::RevealInspected {
            target,
            field,
            origins,
        } => value(
            &service
                .reveal_inspected(session, &target, &field, &origins)?
                .value,
        )?,
        Command::PrepareLifecycle {
            action,
            target,
            destination,
        } => value(
            &service
                .prepare_lifecycle(session, action, target, destination)?
                .value,
        )?,
        Command::ConfirmLifecycle {
            prepared,
            operation,
        } => value(
            &service
                .confirm_lifecycle(session, &prepared, &operation)?
                .value,
        )?,
        Command::MoveGroup { request, operation } => {
            value(&service.move_group(session, &request, &operation)?.value)?
        }
        Command::MoveEntry {
            entry,
            group,
            review,
            operation,
        } => value(
            &service
                .move_entry_to(session, &entry, group, review, &operation)?
                .value,
        )?,
        Command::CloneGroup {
            group,
            parent,
            name,
            operation,
        } => value(
            &service
                .clone_group(session, &group, parent, name, &operation)?
                .value,
        )?,
        Command::PendingSources => value(&service.pending_sources(session)?.value)?,
        Command::RecoverSource { request, operation } => {
            value(&service.recover_source(session, &request, &operation)?.value)?
        }
        Command::ResolveGeneration {
            address,
            heads,
            operation,
        } => value(
            &service
                .resolve_generation(session, &address, &heads, &operation)?
                .value,
        )?,
        Command::DatabaseInfo => value(&service.database_info(session)?)?,
        Command::SetDatabaseInfo {
            name,
            description,
            operation,
        } => {
            service.update_database_info(session, name, description, &operation)?;
            Value::Null
        }
        Command::GroupInfo => value(&service.group_info(session)?)?,
        Command::SaveGroup { form, operation } => {
            value(&service.save_group_form(session, form, &operation)?)?
        }
        Command::EditorView => value(&service.editor_view(session)?)?,
        Command::Drafts => value(&service.drafts(session)?.value)?,
        Command::ResumeDraft(id) => value(&service.resume_draft(session, &id)?.value)?,
        Command::DeleteDraft(id) => {
            service.delete_draft(session, &id)?;
            Value::Null
        }
        Command::PersistDrafts => {
            service.persist_drafts(session)?;
            Value::Null
        }
        Command::SaveDraftSnapshot {
            draft,
            revision,
            operation,
        } => value(
            &service
                .save_draft_snapshot(session, &draft, revision, &operation)?
                .value,
        )?,
        Command::BeginCreateUngrouped => {
            service.start_create_entry_ungrouped(session)?;
            Value::Null
        }
        Command::BeginEditGroup(group) => value(&service.start_edit_group(session, &group)?.value)?,
        Command::BeginCreateGroup(parent) => {
            value(&service.start_create_group(session, parent)?.value)?
        }
        Command::BeginEditDatabaseInfo => value(&service.start_edit_database_info(session)?.value)?,
        Command::GroupHistory(group) => value(&service.group_history(session, &group)?.value)?,
        Command::DatabaseHistory => value(&service.database_history(session)?.value)?,
        Command::PurgeGroupHistory {
            group,
            revisions,
            operation,
        } => {
            service.purge_group_history(
                session,
                &group,
                revisions.into_iter().collect(),
                &operation,
            )?;
            Value::Null
        }
        Command::PurgeDatabaseHistory {
            revisions,
            operation,
        } => {
            service.purge_database_history(session, revisions.into_iter().collect(), &operation)?;
            Value::Null
        }
        Command::MetadataDraft(id) => value(&service.metadata_draft(session, &id)?.value)?,
        Command::PatchGroupDraft { draft, patch } => {
            value(&service.patch_group_draft(session, &draft, patch)?.value)?
        }
        Command::PatchDatabaseDraft { draft, patch } => {
            value(&service.patch_database_draft(session, &draft, patch)?.value)?
        }
        Command::PatchAttribute { patch, remove } => {
            service.patch_attribute(session, patch, remove)?;
            Value::Null
        }
        Command::DraftExpiry(input) => {
            service.set_draft_expiry_input(session, input)?;
            Value::Null
        }
        Command::RevealEditor(attribute) => {
            value(&service.reveal_editor(session, attribute.as_ref())?.expose())?
        }
        Command::RevealRevision {
            entry,
            revision,
            attribute,
        } => {
            let secret = match attribute {
                Some(attribute) => {
                    service
                        .reveal_revision_attribute(session, &entry, &revision, &attribute)?
                        .value
                }
                None => {
                    service
                        .reveal_revision_password(session, &entry, &revision)?
                        .value
                }
            };
            value(&secret.expose())?
        }
        Command::Groups => value(&service.groups(session)?.value)?,
        Command::CreateGroup {
            name,
            parent,
            operation,
        } => value(
            &service
                .create_group(session, name, parent, &operation)?
                .value,
        )?,
        Command::RenameGroup {
            id,
            name,
            operation,
        } => value(&service.update_group(session, &id, name, &operation)?.value)?,
        Command::Entries { group, query } => {
            value(&service.entries(session, group.as_ref(), &query)?.value)?
        }
        Command::Entry(id) => value(&service.view_entry(session, &id)?.value)?,
        Command::CreateEntry {
            group,
            patch,
            operation,
        } => value(
            &service
                .create_entry_in(session, group, patch, &operation)?
                .value,
        )?,
        Command::UpdateEntry {
            id,
            patch,
            operation,
        } => value(&service.update_entry(session, &id, patch, &operation)?.value)?,
        Command::BeginCreate(group) => {
            service.start_create_entry(session, group)?;
            Value::Null
        }
        Command::BeginEdit(id) => {
            service.start_edit_entry(session, &id)?;
            Value::Null
        }
        Command::PatchDraft(patch) => {
            service.patch_draft(session, patch)?;
            Value::Null
        }
        Command::DraftStatus => {
            let draft = service.draft(session)?.value;
            json!({"active": draft.is_some(), "dirty": draft.as_ref().is_some_and(|d| d.dirty),
                "pending": service.pending_draft(session)?.value})
        }
        Command::SaveDraft { operation } => value(&service.save_draft(session, &operation)?.value)?,
        Command::DiscardDraft => {
            service.cancel_draft(session)?;
            Value::Null
        }
        Command::RestoreDraft => {
            service.restore_draft(session)?;
            Value::Null
        }
        Command::History(entry) => value(&service.history(session, &entry)?.value)?,
        Command::Revision { entry, revision } => {
            value(&service.revision(session, &entry, &revision)?.value)?
        }
        Command::CloneEntry {
            entry,
            group,
            title,
            operation,
        } => value(
            &service
                .clone_entry_in(session, &entry, group, title, &operation)?
                .value,
        )?,
        Command::RestoreRevision {
            entry,
            revision,
            group,
            operation,
        } => value(
            &service
                .restore_revision_in(session, &entry, &revision, group, &operation)?
                .value,
        )?,
        Command::PurgeHistory {
            entry,
            revisions,
            operation,
        } => {
            service.purge_history(session, &entry, revisions, &operation)?;
            Value::Null
        }
        Command::Conflicts(entry) => value(&service.conflicts(session, &entry)?.value)?,
        Command::ResolveConflicts {
            context,
            fields,
            operation,
        } => value(
            &service
                .resolve_conflicts(session, &context, fields, &operation)?
                .value,
        )?,
        Command::RevealConflict {
            entry,
            field,
            origins,
        } => value(
            &service
                .reveal_conflict_variant(session, &entry, &field, &origins)?
                .value,
        )?,
        Command::RevealPassword(entry) => {
            value(&service.reveal_password(session, &entry)?.value.expose())?
        }
        Command::RevealAttribute { entry, attribute } => value(
            &service
                .reveal_attribute(session, &entry, &attribute)?
                .value
                .expose(),
        )?,
        Command::BindInvitation { .. } => return Err(RuntimeError::Protocol),
        Command::Lock => {
            service.lock(session)?;
            Value::Null
        }
    })
}

fn unix_seconds() -> Result<u64, RuntimeError> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| RuntimeError::Protocol)
}
