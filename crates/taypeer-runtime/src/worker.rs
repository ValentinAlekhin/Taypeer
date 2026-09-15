//! Child-side dispatch. Only this process constructs a plaintext database service.

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
    let channel = Arc::new(Mutex::new(Channel::new(reader, writer)));
    let mut boot: Boot = channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .read()?;
    if let Some(invitation) = boot.invitation.take() {
        let result = (|| {
            let profile = crate::profile::NativeProfile::load(&boot.profile)?;
            let author = profile.author()?;
            let identity =
                taypeer_trust::Identity::new(author.public(), profile.transport_public())
                    .map_err(|_| RuntimeError::Protocol)?;
            let proof = taypeer_trust::JoinProof::sign(&invitation, identity, &author)
                .map_err(|_| RuntimeError::Protocol)?;
            value(&proof)
        })();
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(result)?;
        return Ok(());
    }
    let mut service = DatabaseService::new();
    let opened = (|| {
        let profile = crate::profile::NativeProfile::load(&boot.profile)?;
        let seed = match boot.create_name.take() {
            Some(name) => {
                let author = profile.author()?;
                let identity =
                    taypeer_trust::Identity::new(author.public(), profile.transport_public())
                        .map_err(|_| RuntimeError::Protocol)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| RuntimeError::Protocol)?
                    .as_millis();
                Some(DatabaseService::prepare_managed(
                    name,
                    boot.password.as_bytes(),
                    &author,
                    identity,
                    now.try_into().map_err(|_| RuntimeError::Protocol)?,
                    Default::default(),
                )?)
            }
            None => None,
        };
        let port = RemotePersistence::attach(
            Arc::clone(&channel),
            boot.path.clone(),
            boot.spool.clone(),
            seed,
        )?;
        service
            .open_managed(Box::new(port), boot.password.as_bytes(), || {
                profile
                    .author()
                    .map(Some)
                    .map_err(|_| taypeer_services::ServiceError::Credentials)
            })
            .map_err(RuntimeError::from)
    })();
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
    let outcome = serve(&channel, &mut service, &session, &boot);
    // EOF and protocol errors also revoke access. A broken parent cannot receive
    // a draft failure, but the process must still stop holding the document.
    let closed = service.lock_all_checked().map_err(RuntimeError::from);
    outcome.and(closed)
}

fn serve(
    channel: &Arc<Mutex<Channel>>,
    service: &mut DatabaseService,
    session: &SessionToken,
    boot: &Boot,
) -> Result<(), RuntimeError> {
    loop {
        let command: Command = channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .read()?;
        let lock = matches!(command, Command::Lock);
        let result = match command {
            Command::RecoverTrust {
                path,
                operation,
                password,
            } => recover(service, session, channel, boot, &path, operation, &password),
            command => dispatch(service, session, command),
        };
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(result)?;
        if lock {
            return Ok(());
        }
    }
}

fn recover(
    service: &DatabaseService,
    session: &SessionToken,
    channel: &Arc<Mutex<Channel>>,
    boot: &Boot,
    path: &std::path::Path,
    operation: taypeer_trust::Digest,
    password: &[u8],
) -> Result<Value, RuntimeError> {
    use crate::cipher_ipc::{IoRequest, IoValue, Seed, spool_objects};
    let profile = crate::profile::NativeProfile::load(&boot.profile)?;
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
                .extract_received(session, &change, &entry, group, &operation)?
                .value,
        )?,
        Command::Authority => value(&service.authority(session)?)?,
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
                .move_entry(session, &entry, group, review, &operation)?
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
        Command::Groups => value(&service.groups(session)?.value)?,
        Command::CreateGroup { name, parent } => {
            value(&service.create_group(session, name, parent)?.value)?
        }
        Command::RenameGroup { id, name } => {
            value(&service.update_group(session, &id, name)?.value)?
        }
        Command::Entries { group, query } => {
            value(&service.entries(session, group.as_ref(), &query)?.value)?
        }
        Command::Entry(id) => value(&service.view_entry(session, &id)?.value)?,
        Command::CreateEntry { group, patch } => {
            service.start_create_entry(session, group)?;
            service.patch_draft(session, patch)?;
            value(&service.save_draft(session)?.value)?
        }
        Command::UpdateEntry { id, patch } => {
            service.start_edit_entry(session, &id)?;
            service.patch_draft(session, patch)?;
            value(&service.save_draft(session)?.value)?
        }
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
        Command::SaveDraft => value(&service.save_draft(session)?.value)?,
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
                .clone_entry(session, &entry, group, title, &operation)?
                .value,
        )?,
        Command::RestoreRevision {
            entry,
            revision,
            group,
            operation,
        } => value(
            &service
                .restore_revision(session, &entry, &revision, group, &operation)?
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
