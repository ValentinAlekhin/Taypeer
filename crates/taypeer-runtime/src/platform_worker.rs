//! Descriptor-backed document processes share service dispatch and supervision with desktop.
use crate::{RuntimeError, Worker, cipher_ipc::Channel, host::HostContext, process::Client};
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use taypeer_services::DatabaseService;
use taypeer_storage::{
    ArchiveSeed, ArchiveSnapshot, CipherPersistence, EncryptedObject, PreparedCommit,
    TemporaryStorage,
};
use taypeer_trust::{AuthorKey, Digest, PublicKey};
use zeroize::Zeroize;

pub mod biometry;
mod credentials;
mod key_input;
mod recovery;
mod recovery_read;
pub use key_input::read_private_key_input;

/// Ciphertext-only bootstrap selected from one protected recovery operation.
pub struct PlatformRecoveryBootstrap {
    /// Exact original local source binding.
    pub source: crate::RecoverySource,
    /// Immutable original ciphertext; never a mutable source-path substitution.
    pub original: ArchiveSnapshot,
    /// Existing encrypted candidate, if one was actually published.
    pub candidate: Option<ArchiveSnapshot>,
    /// Current protected host-profile identity, required when a candidate exists.
    pub identity: Option<taypeer_trust::Identity>,
}

/// Isolated-process access to its host's ciphertext writer. No paths, Keystore or
/// read keys cross this contract. Implementations transfer immutable descriptor leases.
pub trait PlatformDocument: CipherPersistence + Sync {
    /// Production adapters use epoch-encrypted local credentials. The alternate
    /// path exists only for explicitly constructed synthetic process fixtures.
    fn portable_credentials(&self) -> bool {
        true
    }
    /// Read bounded opaque credentials for this exact database lineage.
    fn credential(
        &self,
        _scope: &taypeer_storage::LocalCredentialScope,
        _prepared: bool,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Publish only worker-encrypted credentials; the host never receives a read key.
    fn publish_credential(
        &self,
        _scope: &taypeer_storage::LocalCredentialScope,
        _cipher: &[u8],
        _prepared: bool,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Confirm the prepared epoch after the matching database commit.
    fn finalize_credential(
        &self,
        _scope: &taypeer_storage::LocalCredentialScope,
        _promote_prepared: bool,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Activate only ciphertext transport and independently protected host state.
    fn activate_credentials(
        &self,
        _capability: crate::profile::TransportCapability,
        _authenticated: Option<Digest>,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Recover the exact source workflow through an independently authenticated target.
    fn activate_recovery_credentials(
        &self,
        _source: &crate::RecoverySource,
        _capability: crate::profile::TransportCapability,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Explicit allocator for ciphertext in an isolated UID; no global fallback.
    fn temporary(&self) -> TemporaryStorage;
    /// Publish initial encrypted objects prepared by this worker.
    fn create(&self, seed: ArchiveSeed) -> Result<(), RuntimeError>;
    /// Explicit synthetic-fixture author adapter. Product adapters use encrypted
    /// credentials instead and never expose this through Binder or UniFFI.
    fn author(&self, _authenticated: Option<Digest>) -> Result<Option<AuthorKey>, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Public identity of the host transport. It conveys no author authority.
    fn transport_public(&self) -> Result<PublicKey, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Select the protected immutable source of an existing recovery before password authentication.
    /// Only a missing operation is `None`; a vanished pre-publication attempt requires a new operation.
    fn recovery_bootstrap(
        &self,
        _operation: Digest,
    ) -> Result<Option<PlatformRecoveryBootstrap>, RuntimeError> {
        Ok(None)
    }
    /// Durably capture the exact authenticated ciphertext generation for one recovery.
    /// An existing operation returns its original snapshot, never the latest mutable path.
    fn freeze_recovery(
        &self,
        _operation: Digest,
        _authenticated: Digest,
    ) -> Result<(crate::RecoverySource, ArchiveSnapshot), RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Private recovery-only author capability bound to the exact frozen source.
    /// It never relaxes ordinary author, document-writing or network authority guards.
    fn author_recovery(&self, _source: &crate::RecoverySource) -> Result<AuthorKey, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Read the actual immutable encrypted candidate. Only a missing file is `None`.
    fn recovery_candidate(
        &self,
        _intent: &crate::RecoveryIntent,
    ) -> Result<Option<ArchiveSnapshot>, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Publish a separate encrypted archive without activating its catalog or network.
    fn publish_recovery(
        &self,
        _intent: &crate::RecoveryIntent,
        _seed: ArchiveSeed,
    ) -> Result<crate::RecoveryPublication, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Publish a fresh trust set together with its epoch-encrypted local authority.
    /// Only independent transport state reaches the host; author/read keys stay here.
    fn publish_recovery_credentials(
        &self,
        intent: &crate::RecoveryIntent,
        seed: ArchiveSeed,
        _cipher: Vec<u8>,
        _capability: crate::profile::TransportCapability,
    ) -> Result<crate::RecoveryPublication, RuntimeError> {
        self.publish_recovery(intent, seed)
    }
    /// Confirm the worker-authenticated candidate, real file durability and protected proof.
    fn confirm_recovery(
        &self,
        _publication: &crate::RecoveryPublication,
    ) -> Result<(), RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Consume a selected plaintext input capability for this worker generation.
    /// Ordinary ciphertext persistence adapters have no selected-file access.
    fn selected_input(&self, _id: u64) -> Result<Box<dyn Read + Send>, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
    /// Consume a selected output capability for this worker generation.
    /// Its owner must revoke it on lock and discard incomplete publication.
    fn selected_output(&self, _id: u64) -> Result<Box<dyn SelectedOutput>, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
}

/// A selected plaintext output whose explicit finish confirms completion.
/// File providers must confirm durable publication; an ephemeral bounded preview
/// sink completes its memory result without claiming an external saved file.
/// Dropping either stream is cleanup and must never report operation success.
pub trait SelectedOutput: Write + Send {
    /// Flush, synchronize and complete this selected output, or return a typed failure.
    fn finish(&mut self) -> Result<(), RuntimeError>;
}

#[derive(Serialize, Deserialize)]
struct PlatformBoot {
    password: String,
    form: Option<taypeer_services::CreateDatabase>,
    #[serde(default)]
    invitation: Option<taypeer_trust::Invitation>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    recovery: Option<Digest>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    biometry: Option<biometry::BiometricInputLease>,
}
impl Drop for PlatformBoot {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}
struct Persistence(Arc<dyn PlatformDocument>);
impl CipherPersistence for Persistence {
    fn snapshot(&self) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.0.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.0.commit(request)
    }
    fn working_copy(&self) -> Digest {
        self.0.working_copy()
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), taypeer_storage::Error> {
        self.0.save_draft(object)
    }
    fn load_draft(
        &self,
        chain: &taypeer_trust::ControlChain,
    ) -> Result<Option<EncryptedObject>, taypeer_storage::Error> {
        self.0.load_draft(chain)
    }
    fn discard_draft(&self) -> Result<(), taypeer_storage::Error> {
        self.0.discard_draft()
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
}

/// Run exactly one document generation on private platform streams. This entry
/// point constructs plaintext only inside the isolated process. EOF revokes access.
pub fn run_platform_worker(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    document: Arc<dyn PlatformDocument>,
) -> Result<(), RuntimeError> {
    run_platform_worker_inner(reader, writer, document, None)
}

/// Run a worker with its separate private one-shot key-FD owner.
/// Only the fixed isolated Native bootstrap installs this owner; it is unrelated
/// to ordinary streams, selected files, ciphertext callbacks or credential ports.
pub fn run_platform_worker_with_biometry(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    document: Arc<dyn PlatformDocument>,
    inputs: Arc<dyn biometry::BiometricInputs>,
) -> Result<(), RuntimeError> {
    run_platform_worker_inner(reader, writer, document, Some(inputs))
}

fn run_platform_worker_inner(
    reader: impl Read + Send + 'static,
    writer: impl Write + Send + 'static,
    document: Arc<dyn PlatformDocument>,
    inputs: Option<Arc<dyn biometry::BiometricInputs>>,
) -> Result<(), RuntimeError> {
    let channel = Arc::new(Mutex::new(Channel::new(reader, writer)));
    let mut boot: PlatformBoot = channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .read()?;
    if boot.biometry.is_some()
        && (!boot.password.is_empty()
            || boot.form.is_some()
            || boot.invitation.is_some()
            || boot.recovery.is_some())
    {
        boot.password.zeroize();
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(Err(RuntimeError::Protocol))?;
        return Ok(());
    }
    if let Some(invitation) = boot.invitation.take() {
        // Enrollment signs only public identities. The bearer code, network and
        // Keystore remain with the host; no database or read key is opened here.
        let mut enrollment = None;
        let proof = (|| {
            let author = if document.portable_credentials() {
                let record = credentials::enrollment(
                    document.as_ref(),
                    &invitation,
                    boot.password.as_bytes(),
                )?;
                let author = record.author().ok_or(RuntimeError::Protocol)?;
                enrollment = Some(record);
                author
            } else {
                document.author(None)?.ok_or(RuntimeError::Protocol)?
            };
            let transport = enrollment
                .as_ref()
                .map(|record| record.transport().public())
                .map(Ok)
                .unwrap_or_else(|| document.transport_public())?;
            let identity = taypeer_trust::Identity::new(author.public(), transport)
                .map_err(|_| RuntimeError::Protocol)?;
            let proof = taypeer_trust::JoinProof::sign(&invitation, identity, &author)
                .map_err(|_| RuntimeError::Protocol)?;
            serde_json::to_value(proof).map_err(|_| RuntimeError::Protocol)
        })();
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(proof)?;
        loop {
            let command: crate::Command = channel
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .read()?;
            let result = match command {
                crate::Command::Lock => {
                    boot.password.zeroize();
                    channel
                        .lock()
                        .map_err(|_| RuntimeError::Transport)?
                        .response(Ok(serde_json::Value::Null))?;
                    return Ok(());
                }
                crate::Command::BindInvitation { password, .. } if enrollment.is_some() => {
                    let password = if password.is_empty() {
                        boot.password.as_bytes()
                    } else {
                        &password
                    };
                    credentials::bind_enrollment(
                        document.as_ref(),
                        enrollment.as_mut().ok_or(RuntimeError::Protocol)?,
                        password,
                    )
                    .map(|()| serde_json::Value::Null)
                }
                _ => Err(RuntimeError::Protocol),
            };
            channel
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .response(result)?;
        }
    }
    let recovery_operation = boot.recovery;
    let mut service = DatabaseService::new();
    let mut recovery = None;
    let mut credential = None;
    let opened = if let Some(lease) = boot.biometry.take() {
        match biometry::open_keyed(
            &channel,
            &mut service,
            Arc::clone(&document),
            inputs.as_deref(),
            lease,
            &mut credential,
        ) {
            Ok(Some(session)) => Ok(session),
            Ok(None) => return Ok(()),
            Err(error) => Err(error),
        }
    } else {
        (|| {
            if let Some(form) = boot.form.take() {
                let author = if document.portable_credentials() {
                    AuthorKey::generate().map_err(|_| RuntimeError::Protocol)?
                } else {
                    document.author(None)?.ok_or(RuntimeError::Protocol)?
                };
                let transport = if document.portable_credentials() {
                    Some(
                        taypeer_trust::TransportKey::generate()
                            .map_err(|_| RuntimeError::Protocol)?,
                    )
                } else {
                    None
                };
                let public = match &transport {
                    Some(transport) => transport.public(),
                    None => document.transport_public()?,
                };
                let identity = taypeer_trust::Identity::new(author.public(), public)
                    .map_err(|_| RuntimeError::Protocol)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| RuntimeError::Protocol)?
                    .as_millis();
                let (seed, key) = DatabaseService::prepare_managed_form_with_key_in(
                    form,
                    boot.password.as_bytes(),
                    &author,
                    identity,
                    now.try_into().map_err(|_| RuntimeError::Protocol)?,
                    document.temporary(),
                )?;
                if let Some(transport) = transport {
                    credential = Some(credentials::create(
                        document.as_ref(),
                        &seed,
                        &key,
                        &author,
                        &transport,
                    )?);
                }
                document.create(seed)?;
            }
            open_password(
                &mut service,
                &document,
                &boot.password,
                recovery_operation,
                &mut recovery,
                &mut credential,
            )
        })()
    };
    boot.password.zeroize();
    let session = match opened {
        Ok(session) => session,
        Err(error) => {
            channel
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .response(Err(error))?;
            service.lock_all_checked()?;
            return Ok(());
        }
    };
    channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .response(serde_json::to_value(&session.database).map_err(|_| RuntimeError::Protocol))?;
    let mut enrollment = inputs.map(biometry::Enrollment::new);
    let outcome = (|| loop {
        let incoming: biometry::Incoming = channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .read()?;
        let locking = matches!(&incoming, biometry::Incoming::Command(crate::Command::Lock));
        let result = match incoming {
            biometry::Incoming::Biometry(envelope) if recovery_operation.is_none() => {
                match enrollment.as_mut() {
                    Some(enrollment) => enrollment.dispatch(&service, &session, envelope.biometry),
                    None => Err(taypeer_services::ServiceError::Unauthorized.into()),
                }
            }
            biometry::Incoming::Biometry(_) => Err(taypeer_services::ServiceError::ReadOnly.into()),
            biometry::Incoming::Command(crate::Command::RotatePassword {
                operation,
                password,
                revoke,
            }) if document.portable_credentials() && recovery_operation.is_none() => {
                match credential.as_mut() {
                    Some(record) => credentials::rotate(
                        &mut service,
                        &session,
                        document.as_ref(),
                        record,
                        operation,
                        &password,
                        revoke,
                    ),
                    None => Err(taypeer_services::ServiceError::ReadOnly.into()),
                }
            }
            biometry::Incoming::Command(crate::Command::SetDatabasePolicy {
                operation,
                policy,
                password,
            }) if document.portable_credentials() && recovery_operation.is_none() => {
                match credential.as_mut() {
                    Some(record) => credentials::policy(
                        &mut service,
                        &session,
                        document.as_ref(),
                        record,
                        operation,
                        policy,
                        password.as_ref().map(|password| password.as_slice()),
                    ),
                    None => Err(taypeer_services::ServiceError::ReadOnly.into()),
                }
            }
            biometry::Incoming::Command(command) => {
                if recovery_operation.is_none()
                    || recovery_read::allowed(&command, recovery_operation)
                {
                    dispatch_platform(
                        &mut service,
                        &session,
                        command,
                        document.as_ref(),
                        &mut recovery,
                    )
                } else {
                    Err(taypeer_services::ServiceError::ReadOnly.into())
                }
            }
        };
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(result)?;
        if locking {
            return Ok(());
        }
    })();
    outcome.and(service.lock_all_checked().map_err(RuntimeError::from))
}

fn open_password(
    service: &mut DatabaseService,
    document: &Arc<dyn PlatformDocument>,
    password: &str,
    recovery_operation: Option<Digest>,
    recovery: &mut Option<recovery::WorkingRecovery>,
    credential: &mut Option<taypeer_storage::LocalCredential>,
) -> Result<taypeer_services::SessionToken, RuntimeError> {
    let portable = document.portable_credentials();
    let mut bootstrap = if let Some(operation) = recovery_operation {
        document.recovery_bootstrap(operation)?
    } else {
        None
    };
    if portable && recovery_operation.is_some() {
        let selected = match &bootstrap {
            Some(bootstrap) => bootstrap
                .candidate
                .as_ref()
                .unwrap_or(&bootstrap.original)
                .clone(),
            None => document.snapshot().map_err(crate::cipher_ipc::storage)?,
        };
        let key =
            DatabaseService::authenticate_archive_credentials(&selected, password.as_bytes())?;
        if let Some(prior) = bootstrap
            .as_ref()
            .filter(|bootstrap| bootstrap.candidate.is_some())
        {
            let scope = credentials::scope(&selected)?;
            let cipher = document
                .credential(&scope, false)?
                .ok_or(taypeer_services::ServiceError::Credentials)?;
            let target = taypeer_storage::LocalCredential::open(&cipher, &key, &scope)
                .map_err(crate::cipher_ipc::storage)?;
            document
                .activate_recovery_credentials(&prior.source, credentials::capability(&target))?;
        } else {
            *credential = credentials::open(document.as_ref(), &selected, &key)?;
        }
        let verified =
            document.recovery_bootstrap(recovery_operation.ok_or(RuntimeError::Protocol)?)?;
        if let (Some(before), Some(after)) = (&bootstrap, &verified) {
            if before.source != after.source
                || before.original.fingerprint() != after.original.fingerprint()
                || before.candidate.as_ref().map(ArchiveSnapshot::fingerprint)
                    != after.candidate.as_ref().map(ArchiveSnapshot::fingerprint)
            {
                return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Changed));
            }
        } else if bootstrap.is_some() != verified.is_some() {
            return Err(RuntimeError::Protocol);
        }
        bootstrap = verified;
    }
    let port: Box<dyn CipherPersistence> = if recovery_operation.is_some() {
        let snapshot = match &bootstrap {
            Some(bootstrap) => bootstrap
                .candidate
                .as_ref()
                .unwrap_or(&bootstrap.original)
                .clone(),
            None => document
                .snapshot()
                .map_err(taypeer_services::ServiceError::from)?,
        };
        Box::new(recovery_read::ReadPort::new(
            snapshot,
            document.working_copy(),
        ))
    } else {
        Box::new(Persistence(Arc::clone(document)))
    };
    let mut author_failure = None;
    let session =
        service.open_managed_with_credentials(port, password.as_bytes(), |snapshot, key| {
            let result = if let Some(bootstrap) = bootstrap
                .as_ref()
                .filter(|bootstrap| bootstrap.candidate.is_some())
            {
                let identity = bootstrap
                    .identity
                    .ok_or(taypeer_services::ServiceError::InvalidContext)?;
                let authenticated = taypeer_services::authenticate_named_trust_recovery(
                    &bootstrap.original,
                    snapshot,
                    bootstrap.source.operation,
                    bootstrap.source.working_copy,
                    identity,
                    password.as_bytes(),
                )?;
                *recovery = Some(recovery::resume(
                    bootstrap.source.clone(),
                    bootstrap.original.clone(),
                    authenticated.name,
                    password.as_bytes(),
                    bootstrap.identity,
                ));
                if portable {
                    document
                        .freeze_recovery(
                            bootstrap.source.operation,
                            bootstrap.original.fingerprint(),
                        )
                        .map(|_| None)
                } else {
                    document
                        .freeze_recovery(
                            bootstrap.source.operation,
                            bootstrap.original.fingerprint(),
                        )
                        .and_then(|(source, _)| document.author_recovery(&source))
                        .map(Some)
                }
            } else if let Some(operation) = recovery_operation {
                if portable {
                    Ok(None)
                } else {
                    document
                        .freeze_recovery(operation, snapshot.fingerprint())
                        .and_then(|(source, _)| document.author_recovery(&source))
                        .map(Some)
                }
            } else if portable {
                credentials::open(document.as_ref(), snapshot, key).map(|record| {
                    *credential = record;
                    credential
                        .as_ref()
                        .and_then(taypeer_storage::LocalCredential::author)
                })
            } else {
                document.author(Some(snapshot.fingerprint()))
            };
            result.map_err(|error| {
                if portable || recovery_operation.is_some() {
                    author_failure = Some(error);
                }
                match error {
                    RuntimeError::Service(error) => error,
                    _ => taypeer_services::ServiceError::Credentials,
                }
            })
        });
    if let Some(error) = author_failure {
        return Err(error);
    }
    let session = session?;
    if recovery_operation.is_none() && service.can_write(&session)? {
        service.apply_received(&session)?;
    }
    Ok(session)
}

fn dispatch_platform(
    service: &mut DatabaseService,
    session: &taypeer_services::SessionToken,
    command: crate::Command,
    document: &dyn PlatformDocument,
    recovery: &mut Option<recovery::WorkingRecovery>,
) -> Result<serde_json::Value, RuntimeError> {
    if recovery.is_some()
        && !matches!(
            &command,
            crate::Command::RecoverWorkingCopy { .. }
                | crate::Command::Lock
                | crate::Command::DatabaseInfo
                | crate::Command::SessionAuthority
        )
    {
        return Err(taypeer_services::ServiceError::ReadOnly.into());
    }
    match command {
        crate::Command::RecoverWorkingCopy {
            operation,
            name,
            password,
        } => recovery::recover(
            service,
            session,
            document,
            recovery,
            recovery::Request {
                operation,
                name,
                password,
            },
        ),
        crate::Command::Lock => {
            *recovery = None;
            crate::worker::dispatch(service, session, crate::Command::Lock)
        }
        crate::Command::EditorView => {
            let mut view = service.editor_view(session)?;
            // Android acquires original image bytes only through a bounded
            // selected output. Expanding those bytes into JSON is neither a
            // metadata projection nor compatible with the private frame limit.
            view.icon_preview = None;
            serde_json::to_value(view).map_err(|_| RuntimeError::Protocol)
        }
        crate::Command::ImportSelectedEntryFavicon {
            request,
            input,
            length,
        } => {
            // Do not acquire even a disappearing selected provider on an ACK replay.
            if service.favicon_entry_receipt(session, &request)?.value {
                return Ok(serde_json::Value::Null);
            }
            if length > taypeer_core::ICON_LIMIT {
                return Err(taypeer_services::ServiceError::Icon(
                    taypeer_services::icons::IconError::TooLarge,
                )
                .into());
            }
            service.import_entry_favicon_stream(
                session,
                &request,
                document.selected_input(input).map_err(|_| {
                    taypeer_services::ServiceError::Icon(taypeer_services::icons::IconError::File)
                })?,
                length,
            )?;
            Ok(serde_json::Value::Null)
        }
        crate::Command::ImportSelectedIcon {
            draft,
            input,
            length,
            source,
            operation,
        } => {
            if let Some(identity) = service
                .icon_import_receipt(session, &draft, &source, &operation)?
                .value
            {
                return serde_json::to_value(identity).map_err(|_| RuntimeError::Protocol);
            }
            if length > taypeer_core::ICON_LIMIT {
                return Err(taypeer_services::ServiceError::Icon(
                    taypeer_services::icons::IconError::TooLarge,
                )
                .into());
            }
            let identity = service
                .import_icon_stream(
                    session,
                    &draft,
                    document.selected_input(input).map_err(|_| {
                        taypeer_services::ServiceError::Icon(
                            taypeer_services::icons::IconError::File,
                        )
                    })?,
                    taypeer_services::IconImport { length, source },
                    &operation,
                )?
                .value;
            serde_json::to_value(identity).map_err(|_| RuntimeError::Protocol)
        }
        crate::Command::ImportSelectedAttachment {
            draft,
            input,
            length,
            name,
            replacement,
            operation,
        } => {
            service.import_attachment_stream(
                session,
                &draft,
                document.selected_input(input)?,
                taypeer_services::AttachmentImport {
                    length,
                    name,
                    replacement,
                },
                &operation,
            )?;
            Ok(serde_json::Value::Null)
        }
        crate::Command::ExportSelectedBinary {
            target,
            blob,
            output,
        } => {
            service.binary_export_length(session, &target, &blob)?;
            let mut output = document.selected_output(output)?;
            service.export_binary_stream(session, &target, &blob, &mut output)?;
            output.finish()?;
            Ok(serde_json::Value::Null)
        }
        command => crate::worker::dispatch(service, session, command),
    }
}

impl crate::RuntimeHost {
    /// Content-free process observation for retaining an interrupted enrollment's
    /// platform resources; this never proves database or network authorization.
    pub fn platform_enrollment_active(&self, request: Digest) -> Result<bool, RuntimeError> {
        Ok(self
            .enrollments
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .get(&request)
            .is_some_and(|client| client.control.check(false).is_ok()))
    }
    /// Open a platform worker whose descriptor port is already bound by its launcher.
    /// The launcher must stop it on host death and offer independent process control.
    pub fn open_platform_worker(
        &self,
        launcher: &dyn crate::platform::ProcessLauncher,
        password: String,
        form: Option<taypeer_services::CreateDatabase>,
    ) -> Result<Worker, RuntimeError> {
        self.open_platform_with_recovery(launcher, password, form, None)
    }

    /// Open a constrained authenticated read/recovery process, including a stored control fork.
    /// This does not start application, network exchange or ordinary old-chain author commands.
    pub fn open_platform_recovery_worker(
        &self,
        launcher: &dyn crate::platform::ProcessLauncher,
        password: String,
        operation: Digest,
    ) -> Result<Worker, RuntimeError> {
        self.open_platform_with_recovery(launcher, password, None, Some(operation))
    }

    fn open_platform_with_recovery(
        &self,
        launcher: &dyn crate::platform::ProcessLauncher,
        password: String,
        form: Option<taypeer_services::CreateDatabase>,
        recovery: Option<Digest>,
    ) -> Result<Worker, RuntimeError> {
        let client = Client::connect_platform(launcher.launch()?, &self.sessions)?;
        let mut boot = PlatformBoot {
            password,
            form,
            invitation: None,
            recovery,
            biometry: None,
        };
        let opened = client.request(&boot, true);
        boot.password.zeroize();
        let database = opened
            .and_then(|value| serde_json::from_value(value).map_err(|_| RuntimeError::Protocol))
            .inspect_err(|_| {
                client
                    .control
                    .invalidate(crate::session::LockReason::Transport)
            })?;
        if recovery.is_some() {
            return Worker::from_recovery_client(
                client,
                database,
                self.runtime.handle(),
                &self.sessions,
            );
        }
        let worker = Worker::from_client(
            client,
            database,
            Arc::clone(&self.context),
            self.runtime.handle(),
            &self.sessions,
        )?;
        self.start_requested_network();
        Ok(worker)
    }

    /// Sign a recipient proof in a fresh isolated process, then confirm its exit.
    /// This explicit enrollment operation acquires no password or document read key.
    pub fn platform_join_proof(
        &self,
        launcher: &dyn crate::platform::ProcessLauncher,
        invitation: taypeer_trust::Invitation,
    ) -> Result<taypeer_trust::JoinProof, RuntimeError> {
        let request = invitation.id().map_err(|_| RuntimeError::Protocol)?;
        let proof = self.platform_enroll(launcher, invitation, String::new())?;
        let client = self
            .enrollments
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .remove(&request)
            .ok_or(RuntimeError::Closed)?;
        client
            .control
            .invalidate(crate::session::LockReason::Manual);
        client.control.wait_closed()?;
        Ok(proof)
    }

    /// Retain a password-authenticated enrollment generation until receipt binding.
    pub fn platform_enroll(
        &self,
        launcher: &dyn crate::platform::ProcessLauncher,
        invitation: taypeer_trust::Invitation,
        password: String,
    ) -> Result<taypeer_trust::JoinProof, RuntimeError> {
        let database = invitation.database.clone();
        let request = invitation.id().map_err(|_| RuntimeError::Protocol)?;
        let client = Client::connect_platform(launcher.launch()?, &self.sessions)?;
        let boot = PlatformBoot {
            password,
            form: None,
            invitation: Some(invitation),
            recovery: None,
            biometry: None,
        };
        let proof = client
            .request(&boot, true)
            .and_then(|value| serde_json::from_value(value).map_err(|_| RuntimeError::Protocol))
            .inspect_err(|_| {
                client
                    .control
                    .invalidate(crate::session::LockReason::Transport)
            })?;
        client.control.opened(database)?;
        self.enrollments
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .insert(request, client);
        Ok(proof)
    }

    /// Bind a platform transfer adapter to one host-owned working path. Opening
    /// ciphertext never acquires author credentials or decrypts document content.
    pub fn platform_cipher_writer(
        &self,
        path: &Path,
    ) -> Result<Arc<PlatformCipherWriter>, RuntimeError> {
        let path = crate::host::canonical_path(path)?;
        let lineage = if path.try_exists().map_err(|_| RuntimeError::Transport)? {
            Some(credentials::scope(
                &ArchiveSnapshot::open(&path, None).map_err(crate::cipher_ipc::storage)?,
            )?)
        } else {
            None
        };
        self.platform_cipher_writer_in(path, lineage)
    }

    /// Bind an enrollment port to the invitation chosen by the host before the
    /// recipient file exists. Its isolated worker cannot choose another lineage.
    pub fn platform_enrollment_cipher_writer(
        &self,
        path: &Path,
        invitation: &taypeer_trust::Invitation,
    ) -> Result<Arc<PlatformCipherWriter>, RuntimeError> {
        let path = crate::host::canonical_path(path)?;
        if path.try_exists().map_err(|_| RuntimeError::Transport)? {
            let selected = credentials::scope(
                &ArchiveSnapshot::open(&path, None).map_err(crate::cipher_ipc::storage)?,
            )?;
            if selected.database != invitation.database || selected.root != invitation.root {
                return Err(taypeer_services::ServiceError::Unauthorized.into());
            }
        }
        self.platform_cipher_writer_in(
            path,
            Some(taypeer_storage::LocalCredentialScope {
                database: invitation.database.clone(),
                root: invitation.root,
                epoch: 0,
            }),
        )
    }

    /// Recover only the invitation selected by this durable request. While its
    /// worker is alive the already authenticated local intent permits password
    /// correction; a cold request first authenticates its encrypted staging.
    pub fn platform_pending_join_cipher_writer(
        &self,
        request: Digest,
        password: &[u8],
    ) -> Result<Arc<PlatformCipherWriter>, RuntimeError> {
        let pending = match self.pending_joins()?.remove(&request) {
            Some(pending) => pending,
            None => self.profile().restore_join_request(request, password)?,
        };
        self.platform_enrollment_cipher_writer(&pending.path, &pending.invitation)
    }

    fn platform_cipher_writer_in(
        &self,
        path: PathBuf,
        lineage: Option<taypeer_storage::LocalCredentialScope>,
    ) -> Result<Arc<PlatformCipherWriter>, RuntimeError> {
        let lifetime = self.context.recovery_writers.acquire(&path)?;
        Ok(Arc::new(PlatformCipherWriter {
            context: Arc::clone(&self.context),
            active: Mutex::new(None),
            recovery_target: Mutex::new(None),
            credential_lineage: Mutex::new(lineage),
            recovery_credential: Mutex::new(None),
            path,
            registration: Mutex::new(None),
            authentication: Mutex::new(None),
            recovery_owner: Digest::object(
                b"taypeer.recovery.worker.v1",
                &taypeer_services::new_operation_id()?,
            )
            .map_err(|_| RuntimeError::Protocol)?,
            recovery_lease: Mutex::new(None),
            _recovery_lifetime: lifetime,
        }))
    }
}

/// Ciphertext-only side of a platform document port. The host remains the only
/// file writer; native adapters transfer its snapshots and candidates as leases.
pub struct PlatformCipherWriter {
    pub(crate) context: Arc<HostContext>,
    pub(crate) active: Mutex<Option<Arc<HostContext>>>,
    pub(crate) recovery_target: Mutex<Option<crate::profile::NativeProfile>>,
    credential_lineage: Mutex<Option<taypeer_storage::LocalCredentialScope>>,
    recovery_credential: Mutex<Option<taypeer_storage::LocalCredentialScope>>,
    pub(crate) path: PathBuf,
    registration: Mutex<Option<crate::profile::Registration>>,
    // One issued immutable generation per authentication attempt. Consumption
    // prevents an old digest from becoming a reusable author capability.
    pub(crate) authentication: Mutex<Option<ArchiveSnapshot>>,
    pub(crate) recovery_owner: Digest,
    pub(crate) recovery_lease: Mutex<Option<ArchiveSnapshot>>,
    _recovery_lifetime: Arc<()>,
}
impl PlatformCipherWriter {
    fn check_credential_lineage(
        &self,
        database: &taypeer_core::DatabaseId,
        root: Digest,
        recovery_read: bool,
    ) -> Result<(), RuntimeError> {
        let matches = |scope: &taypeer_storage::LocalCredentialScope| {
            &scope.database == database && scope.root == root
        };
        let mut selected = self
            .credential_lineage
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if selected.is_none()
            && self
                .path
                .try_exists()
                .map_err(|_| RuntimeError::Transport)?
        {
            *selected = Some(credentials::scope(
                &ArchiveSnapshot::open(&self.path, None).map_err(crate::cipher_ipc::storage)?,
            )?);
        }
        if selected.as_ref().is_some_and(matches)
            || (recovery_read
                && self
                    .recovery_credential
                    .lock()
                    .map_err(|_| RuntimeError::Transport)?
                    .as_ref()
                    .is_some_and(matches))
        {
            return Ok(());
        }
        Err(taypeer_services::ServiceError::Unauthorized.into())
    }

    /// Only the host's exact candidate-selection path can extend ciphertext read
    /// access. It grants no publication or normal transport activation authority.
    pub(crate) fn allow_recovery_credential(
        &self,
        candidate: &ArchiveSnapshot,
    ) -> Result<(), RuntimeError> {
        let candidate = credentials::scope(candidate)?;
        let selected = self
            .credential_lineage
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if selected.as_ref().is_none_or(|source| {
            source.database != candidate.database || source.root == candidate.root
        }) {
            return Err(taypeer_services::ServiceError::Unauthorized.into());
        }
        let mut allowed = self
            .recovery_credential
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if allowed.as_ref().is_some_and(|prior| {
            prior.database != candidate.database || prior.root != candidate.root
        }) {
            return Err(taypeer_services::ServiceError::Unauthorized.into());
        }
        *allowed = Some(candidate);
        Ok(())
    }

    /// Activated per-database host state, without author or read-key authority.
    pub(crate) fn active_context(&self) -> Result<Arc<HostContext>, RuntimeError> {
        if !self.context.profile.is_portable() {
            return Ok(Arc::clone(&self.context));
        }
        self.active
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .clone()
            .ok_or(RuntimeError::Closed)
    }
    /// Read only encrypted local credentials, before any host context is activated.
    pub fn credential(
        &self,
        scope: &taypeer_storage::LocalCredentialScope,
        prepared: bool,
    ) -> Result<Option<Vec<u8>>, RuntimeError> {
        self.check_credential_lineage(&scope.database, scope.root, true)?;
        self.context
            .profile
            .read_credential(scope, prepared)
            .map_err(Into::into)
    }
    /// The private worker port accepts ciphertext, never an author seed or database key.
    pub fn publish_credential(
        &self,
        scope: &taypeer_storage::LocalCredentialScope,
        cipher: &[u8],
        prepared: bool,
    ) -> Result<(), RuntimeError> {
        {
            let mut lineage = self
                .credential_lineage
                .lock()
                .map_err(|_| RuntimeError::Transport)?;
            if let Some(selected) = lineage.as_ref() {
                if selected.database != scope.database || selected.root != scope.root {
                    return Err(taypeer_services::ServiceError::Unauthorized.into());
                }
            } else {
                // Creation has no public lineage until its first envelope. It
                // cannot read, stage over, or adopt any pre-existing namespace.
                if prepared
                    || self
                        .path
                        .try_exists()
                        .map_err(|_| RuntimeError::Transport)?
                    || self
                        .context
                        .profile
                        .read_credential(scope, false)?
                        .is_some()
                    || self.context.profile.read_credential(scope, true)?.is_some()
                {
                    return Err(taypeer_services::ServiceError::Unauthorized.into());
                }
            }
            // The exact owner survives an uncertain first publication, so its
            // retry can confirm equal durable bytes without reopening scope.
            *lineage = Some(scope.clone());
            self.context
                .profile
                .publish_credential(scope, cipher, prepared)?;
        }
        if prepared && let Ok(context) = self.active_context() {
            context
                .profile
                .save_state("credential_prepared", &Some(scope.epoch))?;
        }
        Ok(())
    }
    /// Complete the prepared credential and protected epoch markers together.
    pub fn finalize_credential(
        &self,
        scope: &taypeer_storage::LocalCredentialScope,
        promote_prepared: bool,
    ) -> Result<(), RuntimeError> {
        self.check_credential_lineage(&scope.database, scope.root, false)?;
        let context = self.active_context()?;
        if promote_prepared {
            self.context.profile.finalize_credential(scope)?;
        } else {
            self.context.profile.confirm_active_credential(scope)?;
        }
        context
            .profile
            .save_state("credential_final", &scope.epoch)?;
        context
            .profile
            .save_state("credential_prepared", &None::<u64>)?;
        Ok(())
    }
    /// Attach transport only after the isolated worker authenticated this exact generation.
    pub fn activate_credentials(
        &self,
        capability: crate::profile::TransportCapability,
        authenticated: Option<Digest>,
    ) -> Result<(), RuntimeError> {
        self.check_credential_lineage(&capability.database, capability.root, false)?;
        let expected = if let Some(expected) = authenticated {
            let snapshot = ArchiveSnapshot::open(&self.path, Some(capability.root))
                .map_err(crate::cipher_ipc::storage)?;
            if snapshot.fingerprint() != expected
                || snapshot.chain().head().database != capability.database
            {
                return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Changed));
            }
            Some(snapshot)
        } else {
            None
        };
        let context = self.context.activate(capability)?;
        if let Some(expected) = expected {
            let registration = context.attach(&self.path)?;
            let current = context
                .coordinator
                .snapshot(&registration.database)
                .map_err(crate::host::sync_error)?;
            if current.fingerprint() != expected.fingerprint() {
                return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Changed));
            }
            *self
                .registration
                .lock()
                .map_err(|_| RuntimeError::Transport)? = Some(registration);
        }
        *self.active.lock().map_err(|_| RuntimeError::Transport)? = Some(context);
        Ok(())
    }
    pub(crate) fn registration(&self) -> Result<crate::profile::Registration, RuntimeError> {
        let mut registration = self
            .registration
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if registration.is_none() {
            *registration = Some(self.active_context()?.attach(&self.path)?);
        }
        registration.clone().ok_or(RuntimeError::Protocol)
    }
    /// Immutable verified archive generation, without its filesystem writer.
    pub fn snapshot(&self) -> Result<ArchiveSnapshot, RuntimeError> {
        if self.context.profile.is_portable()
            && self
                .active
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .is_none()
        {
            let snapshot =
                ArchiveSnapshot::open(&self.path, None).map_err(crate::cipher_ipc::storage)?;
            *self
                .authentication
                .lock()
                .map_err(|_| RuntimeError::Transport)? = Some(snapshot.clone());
            return Ok(snapshot);
        }
        let registration = self.registration()?;
        let snapshot = self
            .active_context()?
            .coordinator
            .snapshot(&registration.database)
            .map_err(crate::host::sync_error)?;
        *self
            .authentication
            .lock()
            .map_err(|_| RuntimeError::Transport)? = Some(snapshot.clone());
        Ok(snapshot)
    }
    /// Stable local-only draft binding supplied by the protected profile.
    pub fn working_copy(&self) -> Result<Digest, RuntimeError> {
        if self.context.profile.is_portable()
            && self
                .active
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .is_none()
        {
            return Ok(self
                .context
                .profile
                .bootstrap_registration(&self.snapshot()?)?
                .working_copy);
        }
        Ok(self.registration()?.working_copy)
    }
    /// Public transport key for creation; not a credential or admission grant.
    pub fn transport_public(&self) -> PublicKey {
        self.context.transport.public()
    }
    /// Author seed stays on the trusted host-to-worker adapter. The worker calls
    /// this only after authenticating the specified snapshot; it is never a UI API.
    pub fn author(&self, authenticated: Option<Digest>) -> Result<AuthorKey, RuntimeError> {
        let issued = if let Some(expected) = authenticated {
            let snapshot = self
                .authentication
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .take()
                .ok_or_else(|| crate::cipher_ipc::storage(taypeer_storage::Error::Changed))?;
            if snapshot.fingerprint() != expected {
                return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Changed));
            }
            self.check_authenticated_authority(&snapshot)?;
            Some(snapshot)
        } else if self
            .path
            .try_exists()
            .map_err(|_| RuntimeError::Transport)?
        {
            return Err(RuntimeError::Protocol);
        } else {
            None
        };
        let author = self.context.profile.author()?;
        // A native credential prompt can block while a rotation or revocation
        // arrives. Never return its seed after that authority has changed.
        if let Some(snapshot) = &issued {
            self.check_authenticated_authority(snapshot)?;
        }
        Ok(author)
    }

    fn check_authenticated_authority(
        &self,
        snapshot: &ArchiveSnapshot,
    ) -> Result<(), RuntimeError> {
        let state = self
            .context
            .coordinator
            .state(&snapshot.chain().head().database)
            .map_err(crate::host::sync_error)?;
        if state.frozen
            || state.control
                != snapshot
                    .chain()
                    .head_hash()
                    .map_err(|_| RuntimeError::Protocol)?
            || state.epoch != snapshot.chain().head().epoch
        {
            return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Changed));
        }
        Ok(())
    }
    /// Publish a complete encrypted genesis and its protected registration.
    pub fn create(&self, seed: ArchiveSeed) -> Result<(), RuntimeError> {
        if self.context.profile.is_portable() {
            let control = seed.controls.last().ok_or(RuntimeError::Protocol)?;
            let root = seed
                .controls
                .first()
                .ok_or(RuntimeError::Protocol)?
                .hash()
                .map_err(|_| RuntimeError::Protocol)?;
            self.check_credential_lineage(&control.body.database, root, false)?;
        }
        let mut registration = self
            .registration
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if registration.is_some() {
            return Err(RuntimeError::Protocol);
        }
        *registration = Some(self.active_context()?.create(&self.path, seed)?);
        Ok(())
    }
    /// Commit encrypted candidates using the shared serialized coordinator.
    pub fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, RuntimeError> {
        let registration = self.registration()?;
        self.active_context()?
            .coordinator
            .commit(&registration.database, request)
            .map_err(crate::host::sync_error)
    }
    /// Durably retain the encrypted local collection, excluded from P2P.
    pub fn save_draft(&self, object: &EncryptedObject) -> Result<(), RuntimeError> {
        let copy = self.working_copy()?;
        taypeer_storage::save_local_draft(&self.path, copy, object)
            .map_err(crate::cipher_ipc::storage)
    }
    /// Read only the local draft role against the current verified trust chain.
    pub fn load_draft(&self) -> Result<Option<EncryptedObject>, RuntimeError> {
        let snapshot = self.snapshot()?;
        taypeer_storage::read_local_draft_in(
            &self.path,
            self.working_copy()?,
            snapshot.chain(),
            self.active_context()?.ciphertext_staging(),
        )
        .map_err(crate::cipher_ipc::storage)
    }
    /// Explicitly discard the working copy's collection with a durable result.
    pub fn discard_draft(&self) -> Result<(), RuntimeError> {
        taypeer_storage::discard_local_draft(&self.path, self.working_copy()?)
            .map_err(crate::cipher_ipc::storage)
    }
}

#[cfg(test)]
mod tests;
