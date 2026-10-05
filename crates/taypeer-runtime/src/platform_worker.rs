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
}

#[derive(Serialize, Deserialize)]
struct PlatformBoot {
    password: String,
    form: Option<taypeer_services::CreateDatabase>,
    #[serde(default)]
    invitation: Option<taypeer_trust::Invitation>,
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
    let channel = Arc::new(Mutex::new(Channel::new(reader, writer)));
    let mut boot: PlatformBoot = channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .read()?;
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
    let mut service = DatabaseService::new();
    let opened = (|| {
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
        let session = service.open_managed_with_author(
            Box::new(Persistence(Arc::clone(&document))),
            boot.password.as_bytes(),
            |snapshot| {
                document
                    .author(Some(snapshot.fingerprint()))
                    .map_err(|error| match error {
                        RuntimeError::Service(error) => error,
                        _ => taypeer_services::ServiceError::Credentials,
                    })
            },
        )?;
        if service.can_write(&session)? {
            service.apply_received(&session)?;
        }
        Ok::<_, RuntimeError>(session)
    })();
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
    let outcome = (|| loop {
        let command: crate::Command = channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .read()?;
        let locking = matches!(command, crate::Command::Lock);
        let result = crate::worker::dispatch(&mut service, &session, command);
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

impl crate::RuntimeHost {
    /// Open a platform worker whose descriptor port is already bound by its launcher.
    /// The launcher must stop it on host death and offer independent process control.
    pub fn open_platform_worker(
        &self,
        launcher: &dyn crate::platform::ProcessLauncher,
        password: String,
        form: Option<taypeer_services::CreateDatabase>,
    ) -> Result<Worker, RuntimeError> {
        let client = Client::connect_platform(launcher.launch()?, &self.sessions)?;
        let mut boot = PlatformBoot {
            password,
            form,
            invitation: None,
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
        Ok(Arc::new(PlatformCipherWriter {
            context: Arc::clone(&self.context),
            path,
            registration: Mutex::new(None),
            authentication: Mutex::new(None),
        }))
    }
}

/// Ciphertext-only side of a platform document port. The host remains the only
/// file writer; native adapters transfer its snapshots and candidates as leases.
pub struct PlatformCipherWriter {
    context: Arc<HostContext>,
    path: PathBuf,
    registration: Mutex<Option<crate::profile::Registration>>,
    // One issued immutable generation per authentication attempt. Consumption
    // prevents an old digest from becoming a reusable author capability.
    authentication: Mutex<Option<ArchiveSnapshot>>,
}
impl PlatformCipherWriter {
    fn registration(&self) -> Result<crate::profile::Registration, RuntimeError> {
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
