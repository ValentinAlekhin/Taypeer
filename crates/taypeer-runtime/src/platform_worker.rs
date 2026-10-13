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
    /// Explicit allocator for ciphertext in an isolated UID; no global fallback.
    fn temporary(&self) -> TemporaryStorage;
    /// Publish initial encrypted objects prepared by this worker.
    fn create(&self, seed: ArchiveSeed) -> Result<(), RuntimeError>;
    /// Acquire author authority only after snapshot authentication, or explicit creation.
    /// The fingerprint binds the capability to the generation that was authenticated.
    fn author(&self, authenticated: Option<Digest>) -> Result<Option<AuthorKey>, RuntimeError>;
    /// Public identity of the host transport. It conveys no author authority.
    fn transport_public(&self) -> Result<PublicKey, RuntimeError>;
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
        let proof = (|| {
            let author = document.author(None)?.ok_or(RuntimeError::Protocol)?;
            let identity =
                taypeer_trust::Identity::new(author.public(), document.transport_public()?)
                    .map_err(|_| RuntimeError::Protocol)?;
            let proof = taypeer_trust::JoinProof::sign(&invitation, identity, &author)
                .map_err(|_| RuntimeError::Protocol)?;
            serde_json::to_value(proof).map_err(|_| RuntimeError::Protocol)
        })();
        boot.password.zeroize();
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(proof)?;
        let command: crate::Command = channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .read()?;
        if !matches!(command, crate::Command::Lock) {
            return Err(RuntimeError::Protocol);
        }
        channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .response(Ok(serde_json::Value::Null))?;
        return Ok(());
    }
    let recovery_operation = boot.recovery;
    let mut service = DatabaseService::new();
    let mut recovery = None;
    let opened = if let Some(lease) = boot.biometry.take() {
        match biometry::open_keyed(
            &channel,
            &mut service,
            Arc::clone(&document),
            inputs.as_deref(),
            lease,
        ) {
            Ok(Some(session)) => Ok(session),
            Ok(None) => return Ok(()),
            Err(error) => Err(error),
        }
    } else {
        (|| {
            if let Some(form) = boot.form.take() {
                let author = document.author(None)?.ok_or(RuntimeError::Protocol)?;
                let identity =
                    taypeer_trust::Identity::new(author.public(), document.transport_public()?)
                        .map_err(|_| RuntimeError::Protocol)?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| RuntimeError::Protocol)?
                    .as_millis();
                let seed = DatabaseService::prepare_managed_form_in(
                    form,
                    boot.password.as_bytes(),
                    &author,
                    identity,
                    now.try_into().map_err(|_| RuntimeError::Protocol)?,
                    document.temporary(),
                )?;
                document.create(seed)?;
            }
            let bootstrap = if let Some(operation) = recovery_operation {
                document.recovery_bootstrap(operation)?
            } else {
                None
            };
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
                Box::new(Persistence(Arc::clone(&document)))
            };
            let mut author_failure = None;
            let session =
                service.open_managed_with_author(port, boot.password.as_bytes(), |snapshot| {
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
                            boot.password.as_bytes(),
                        )?;
                        recovery = Some(recovery::resume(
                            bootstrap.source.clone(),
                            bootstrap.original.clone(),
                            authenticated.name,
                            boot.password.as_bytes(),
                        ));
                        document
                            .freeze_recovery(
                                bootstrap.source.operation,
                                bootstrap.original.fingerprint(),
                            )
                            .and_then(|(source, _)| document.author_recovery(&source))
                            .map(Some)
                    } else if let Some(operation) = recovery_operation {
                        document
                            .freeze_recovery(operation, snapshot.fingerprint())
                            .and_then(|(source, _)| document.author_recovery(&source))
                            .map(Some)
                    } else {
                        document.author(Some(snapshot.fingerprint()))
                    };
                    result.map_err(|error| {
                        if recovery_operation.is_some() {
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
            Ok::<_, RuntimeError>(session)
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
        Worker::from_client(
            client,
            database,
            Arc::clone(&self.context),
            self.runtime.handle(),
            &self.sessions,
        )
    }

    /// Sign a recipient proof in a fresh isolated process, then confirm its exit.
    /// This explicit enrollment operation acquires no password or document read key.
    pub fn platform_join_proof(
        &self,
        launcher: &dyn crate::platform::ProcessLauncher,
        invitation: taypeer_trust::Invitation,
    ) -> Result<taypeer_trust::JoinProof, RuntimeError> {
        let database = invitation.database.clone();
        let client = Client::connect_platform(launcher.launch()?, &self.sessions)?;
        let boot = PlatformBoot {
            password: String::new(),
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
        client
            .control
            .invalidate(crate::session::LockReason::Manual);
        let outcome = client.control.wait_closed()?;
        if outcome.draft != crate::session::DraftDisposition::Preserved {
            return Err(RuntimeError::ShutdownUnconfirmed);
        }
        Ok(proof)
    }

    /// Bind a platform transfer adapter to one host-owned working path. Opening
    /// ciphertext never acquires author credentials or decrypts document content.
    pub fn platform_cipher_writer(
        &self,
        path: &Path,
    ) -> Result<Arc<PlatformCipherWriter>, RuntimeError> {
        let path = crate::host::canonical_path(path)?;
        let lifetime = self.context.recovery_writers.acquire(&path)?;
        Ok(Arc::new(PlatformCipherWriter {
            context: Arc::clone(&self.context),
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
    pub(crate) fn registration(&self) -> Result<crate::profile::Registration, RuntimeError> {
        let mut registration = self
            .registration
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if registration.is_none() {
            *registration = Some(self.context.attach(&self.path)?);
        }
        registration.clone().ok_or(RuntimeError::Protocol)
    }
    /// Immutable verified archive generation, without its filesystem writer.
    pub fn snapshot(&self) -> Result<ArchiveSnapshot, RuntimeError> {
        let registration = self.registration()?;
        let snapshot = self
            .context
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
        let mut registration = self
            .registration
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if registration.is_some() {
            return Err(RuntimeError::Protocol);
        }
        *registration = Some(self.context.create(&self.path, seed)?);
        Ok(())
    }
    /// Commit encrypted candidates using the shared serialized coordinator.
    pub fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, RuntimeError> {
        let registration = self.registration()?;
        self.context
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
            self.context.ciphertext_staging(),
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
