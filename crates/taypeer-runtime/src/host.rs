//! Host lifetime for native credentials, ciphertext writers and the optional Iroh endpoint.
use crate::{
    RuntimeError, Worker,
    cipher_ipc::{self, IoRequest, IoValue},
    profile::{NativeProfile, ProfileLease, Registration},
};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use taypeer_core::DatabaseId;
use taypeer_storage::{ArchiveSeed, ArchiveSnapshot, ArchiveStore, PreparedCommit};
use taypeer_sync::Coordinator;
use taypeer_trust::{ControlChain, Digest, TransportKey};
use tempfile::NamedTempFile;

/// Platform/CLI host. Writers and native transport identity outlive individual unlocked workers.
pub struct RuntimeHost {
    _lease: ProfileLease,
    pub(crate) context: Arc<HostContext>,
    pub(crate) runtime: tokio::runtime::Runtime,
    pub(crate) network: Option<crate::network::Network>,
}

/// Public format and transport-admission state; querying it never unlocks a database.
#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct RegisteredCompatibility {
    /// Independently assessed format capabilities.
    pub format: taypeer_core::CompatibilityReport,
    /// Whether this profile's transport identity is admitted by the current control.
    pub admitted: bool,
}
pub(crate) struct HostContext {
    pub profile: NativeProfile,
    pub coordinator: Arc<Coordinator>,
    pub transport: Arc<TransportKey>,
    pub copies: Mutex<BTreeMap<PathBuf, Registration>>,
}
impl RuntimeHost {
    /// Verify a standalone file and inspect its public format requirements without
    /// creating a native profile, acquiring credentials, or registering a writer.
    pub fn inspect_compatibility(
        path: &Path,
    ) -> Result<(DatabaseId, taypeer_core::CompatibilityReport), RuntimeError> {
        let snapshot = ArchiveSnapshot::open(path, None).map_err(cipher_ipc::storage)?;
        Ok((
            snapshot.chain().head().database.clone(),
            snapshot.compatibility(&taypeer_core::ClientCapabilities::default()),
        ))
    }
    /// Shared native profile location used by desktop clients and the CLI.
    pub fn default_profile_path() -> Result<PathBuf, RuntimeError> {
        std::env::var_os("HOME")
            .map(|home| {
                PathBuf::from(home).join("Library/Application Support/Taypeer/profiles/default")
            })
            .ok_or(RuntimeError::Profile(
                crate::profile::ProfileError::Credentials,
            ))
    }

    /// Open/create through the same ciphertext writer for an in-process native UI.
    /// Call on a background thread. Author credentials are acquired after password
    /// authentication when opening; creation explicitly enrolls the local author.
    pub fn open_local(
        &self,
        service: &mut taypeer_services::DatabaseService,
        path: &Path,
        password: &[u8],
        name: Option<String>,
    ) -> Result<taypeer_services::SessionToken, RuntimeError> {
        let path = canonical_path(path)?;
        let existed = self
            .context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .contains_key(&path);
        let opened = (|| {
            let registration = if let Some(name) = name {
                let author = self.profile().author()?;
                let identity =
                    taypeer_trust::Identity::new(author.public(), self.context.transport.public())
                        .map_err(|error| {
                            RuntimeError::from(taypeer_services::ServiceError::Trust(error))
                        })?;
                let now = std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)
                    .map_err(|_| RuntimeError::Transport)?
                    .as_millis();
                let now = i64::try_from(now).map_err(|_| RuntimeError::Transport)?;
                let seed = taypeer_services::DatabaseService::prepare_managed(
                    name,
                    password,
                    &author,
                    identity,
                    now,
                    taypeer_core::DatabasePolicy::default(),
                )?;
                self.context.create(&path, seed)?
            } else {
                self.context.attach(&path)?
            };
            let port = taypeer_sync::CoordinatorPersistence::new(
                Arc::clone(self.coordinator()),
                registration.database,
                path.clone(),
                registration.working_copy,
            )
            .map_err(sync_error)?;
            Ok(service.open_managed(Box::new(port), password, || {
                self.profile()
                    .author()
                    .map(Some)
                    .map_err(|_| taypeer_services::ServiceError::Credentials)
            })?)
        })();
        if opened.is_err() && !existed {
            self.context.unregister_path(&path)?;
        }
        opened
    }

    /// Reauthenticate a locked native session without replacing it on failure.
    pub fn unlock_local(
        &self,
        service: &mut taypeer_services::DatabaseService,
        database: &DatabaseId,
        password: &[u8],
    ) -> Result<taypeer_services::SessionToken, RuntimeError> {
        let path = self
            .context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .iter()
            .find_map(|(path, registration)| {
                (&registration.database == database).then_some(path.clone())
            })
            .ok_or(RuntimeError::Closed)?;
        self.open_local(service, &path, password, None)
    }
    /// Start a native profile and its ciphertext coordinator. No networking or author access
    /// occurs until the caller explicitly starts exchange or opens/creates a database.
    pub fn new(profile: &Path) -> Result<Self, RuntimeError> {
        let lease = NativeProfile::acquire(profile)?;
        let profile = lease.profile().clone();
        let transport = Arc::new(profile.transport()?);
        let coordinator = Arc::new(Coordinator::new(Arc::clone(&transport)));
        let runtime = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
            .map_err(|_| RuntimeError::Transport)?;
        Ok(Self {
            _lease: lease,
            network: None,
            context: Arc::new(HostContext {
                profile,
                coordinator,
                transport,
                copies: Mutex::new(BTreeMap::new()),
            }),
            runtime,
        })
    }
    /// Start one plaintext process while retaining ciphertext storage independently.
    pub fn open(
        &self,
        executable: &Path,
        path: &Path,
        password: String,
        name: Option<String>,
    ) -> Result<Worker, RuntimeError> {
        let path = canonical_path(path)?;
        let existed = self
            .context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .contains_key(&path);
        let opened = Worker::open(
            executable,
            &path,
            password,
            name,
            Arc::clone(&self.context),
            self.runtime.handle(),
        );
        if opened.is_err() && !existed {
            self.context.unregister_path(&path)?;
        }
        opened
    }
    /// Release a locked/unlocked catalog item after its worker has been closed.
    pub fn close(&self, database: &DatabaseId) -> Result<(), RuntimeError> {
        let mut copies = self
            .context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        if let Some(path) = copies
            .iter()
            .find_map(|(path, r)| (&r.database == database).then_some(path.clone()))
        {
            self.context
                .coordinator
                .unregister(database)
                .map_err(sync_error)?;
            copies.remove(&path);
        }
        Ok(())
    }
    /// Inspect registered public format/admission state without reading author credentials.
    pub fn compatibility(
        &self,
        database: &DatabaseId,
    ) -> Result<RegisteredCompatibility, RuntimeError> {
        let snapshot = self
            .context
            .coordinator
            .snapshot(database)
            .map_err(sync_error)?;
        Ok(RegisteredCompatibility {
            format: snapshot.compatibility(&taypeer_core::ClientCapabilities::default()),
            admitted: snapshot
                .chain()
                .head()
                .members
                .values()
                .any(|member| member.identity.transport == self.context.transport.public()),
        })
    }
    /// Shared ciphertext coordinator, safe to use while all plaintext workers are stopped.
    pub fn coordinator(&self) -> &Arc<Coordinator> {
        &self.context.coordinator
    }
    /// Nonsecret native profile metadata and explicit credential acquisition adapter.
    pub fn profile(&self) -> &NativeProfile {
        &self.context.profile
    }
}
pub(crate) fn sync_error(error: taypeer_sync::Error) -> RuntimeError {
    match error {
        taypeer_sync::Error::Storage(error) => cipher_ipc::storage(error),
        taypeer_sync::Error::Trust(error) => taypeer_services::ServiceError::Trust(error).into(),
        _ => RuntimeError::Transport,
    }
}
impl HostContext {
    fn unregister_path(&self, path: &Path) -> Result<(), RuntimeError> {
        let mut copies = self.copies.lock().map_err(|_| RuntimeError::Transport)?;
        if let Some(registration) = copies.remove(path) {
            self.coordinator
                .unregister(&registration.database)
                .map_err(sync_error)?;
        }
        Ok(())
    }
    pub fn attach(&self, path: &Path) -> Result<Registration, RuntimeError> {
        let mut copies = self.copies.lock().map_err(|_| RuntimeError::Transport)?;
        if let Some(registration) = copies.get(path) {
            return Ok(registration.clone());
        }
        let prior = self.profile.registration(path)?;
        let snapshot = ArchiveSnapshot::open(path, prior.as_ref().map(|r| r.root))
            .map_err(cipher_ipc::storage)?;
        let registration = self.profile.register(path, &snapshot)?;
        let store = ArchiveStore::open(
            path,
            Some(registration.root),
            Some(self.profile.anchor(&registration)),
        )
        .map_err(cipher_ipc::storage)?;
        self.coordinator.register(store).map_err(sync_error)?;
        copies.insert(path.to_owned(), registration.clone());
        Ok(registration)
    }
    pub fn create(&self, path: &Path, seed: ArchiveSeed) -> Result<Registration, RuntimeError> {
        let mut copies = self.copies.lock().map_err(|_| RuntimeError::Transport)?;
        if copies.contains_key(path) {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Busy));
        }
        if path.try_exists().map_err(|_| RuntimeError::Transport)? {
            return Err(cipher_ipc::storage(taypeer_storage::Error::AlreadyExists));
        }
        let (registration, store) = self.create_store(path, seed)?;
        self.coordinator.register(store).map_err(sync_error)?;
        copies.insert(path.to_owned(), registration.clone());
        Ok(registration)
    }
    fn create_store(
        &self,
        path: &Path,
        seed: ArchiveSeed,
    ) -> Result<(Registration, ArchiveStore), RuntimeError> {
        let root = seed
            .controls
            .first()
            .ok_or(RuntimeError::Protocol)?
            .hash()
            .map_err(|_| RuntimeError::Protocol)?;
        let chain = ControlChain::validate(seed.controls.clone(), root)
            .map_err(|_| RuntimeError::Protocol)?;
        let registration =
            self.profile
                .prepare_registration(path, chain.head().database.clone(), root)?;
        let store = seed
            .create(
                path,
                &self.transport,
                Some(self.profile.anchor(&registration)),
            )
            .map_err(cipher_ipc::storage)?;
        Ok((registration, store))
    }
    fn publish_recovery(&self, path: &Path, seed: ArchiveSeed) -> Result<(), RuntimeError> {
        let path = canonical_path(path)?;
        // The original remains registered. The separate archive becomes active only
        // after an explicit close/open, so one database never has two live writers.
        let (_, store) = self.create_store(&path, seed)?;
        drop(store);
        Ok(())
    }
}
pub(crate) struct Callbacks {
    context: Arc<HostContext>,
    path: PathBuf,
    directory: PathBuf,
    registration: Option<Registration>,
    // Keep reply spools alive until the next callback or completed response. The
    // child has opened a stable inode by then; these files contain ciphertext only.
    spools: Vec<NamedTempFile>,
}
impl Callbacks {
    pub fn new(context: Arc<HostContext>, path: PathBuf, directory: PathBuf) -> Self {
        Self {
            context,
            path,
            directory,
            registration: None,
            spools: Vec::new(),
        }
    }
    pub fn handle(&mut self, request: IoRequest) -> Result<IoValue, RuntimeError> {
        self.spools.clear();
        match request {
            IoRequest::Open => {
                self.registration = Some(self.context.attach(&self.path)?);
                self.snapshot(None)
            }
            IoRequest::Create(seed) => {
                let root = seed
                    .controls
                    .first()
                    .ok_or(RuntimeError::Protocol)?
                    .hash()
                    .map_err(|_| RuntimeError::Protocol)?;
                let chain = ControlChain::validate(seed.controls.clone(), root)
                    .map_err(|_| RuntimeError::Protocol)?;
                let objects = cipher_ipc::read_objects(seed.objects, &self.directory, &chain)?;
                self.registration = Some(self.context.create(
                    &self.path,
                    ArchiveSeed {
                        controls: seed.controls,
                        checkpoint: seed.checkpoint,
                        baseline: seed.baseline,
                        objects,
                    },
                )?);
                self.snapshot(None)
            }
            IoRequest::Recover { path, seed } => {
                let root = seed
                    .controls
                    .first()
                    .ok_or(RuntimeError::Protocol)?
                    .hash()
                    .map_err(|_| RuntimeError::Protocol)?;
                let chain = ControlChain::validate(seed.controls.clone(), root)
                    .map_err(|_| RuntimeError::Protocol)?;
                let objects = cipher_ipc::read_objects(seed.objects, &self.directory, &chain)?;
                self.context.publish_recovery(
                    &path,
                    ArchiveSeed {
                        controls: seed.controls,
                        objects,
                        checkpoint: seed.checkpoint,
                        baseline: seed.baseline,
                    },
                )?;
                Ok(IoValue::Done)
            }
            IoRequest::Snapshot { known } => self.snapshot(known),
            IoRequest::Commit(request) => {
                let registration = self.registration.as_ref().ok_or(RuntimeError::Protocol)?;
                let chain = ControlChain::validate(request.controls.clone(), registration.root)
                    .map_err(|_| RuntimeError::Protocol)?;
                let objects = cipher_ipc::read_objects(request.objects, &self.directory, &chain)?;
                self.context
                    .coordinator
                    .commit(
                        &registration.database,
                        PreparedCommit {
                            expected: request.expected,
                            control: request.control,
                            controls: request.controls,
                            objects,
                            remove: request.remove,
                            checkpoint: request.checkpoint,
                            baseline: request.baseline,
                            journal: request.journal,
                        },
                    )
                    .map_err(sync_error)?;
                self.snapshot(None)
            }
            IoRequest::SaveDraft(path) => {
                let registration = self.registration.as_ref().ok_or(RuntimeError::Protocol)?;
                let snapshot = self
                    .context
                    .coordinator
                    .snapshot(&registration.database)
                    .map_err(sync_error)?;
                let path = cipher_ipc::checked_spool(&path, &self.directory)?;
                let object = taypeer_storage::EncryptedObject::open(&path, snapshot.chain())
                    .map_err(cipher_ipc::storage)?;
                taypeer_storage::save_local_draft(&self.path, registration.working_copy, &object)
                    .map_err(cipher_ipc::storage)?;
                Ok(IoValue::Done)
            }
            IoRequest::LoadDraft => {
                let registration = self.registration.as_ref().ok_or(RuntimeError::Protocol)?;
                let snapshot = self
                    .context
                    .coordinator
                    .snapshot(&registration.database)
                    .map_err(sync_error)?;
                let object = taypeer_storage::read_local_draft(
                    &self.path,
                    registration.working_copy,
                    snapshot.chain(),
                )
                .map_err(cipher_ipc::storage)?;
                let path = if let Some(object) = object {
                    let mut file = NamedTempFile::new_in(&self.directory)
                        .map_err(|_| RuntimeError::Transport)?;
                    std::io::copy(
                        &mut object.reader().map_err(cipher_ipc::storage)?,
                        &mut file,
                    )
                    .map_err(|_| RuntimeError::Transport)?;
                    let path = file.path().to_owned();
                    self.spools.push(file);
                    Some(path)
                } else {
                    None
                };
                Ok(IoValue::Draft(path))
            }
            IoRequest::DiscardDraft => {
                let registration = self.registration.as_ref().ok_or(RuntimeError::Protocol)?;
                taypeer_storage::discard_local_draft(&self.path, registration.working_copy)
                    .map_err(cipher_ipc::storage)?;
                Ok(IoValue::Done)
            }
        }
    }
    fn snapshot(&mut self, known: Option<Digest>) -> Result<IoValue, RuntimeError> {
        let registration = self.registration.as_ref().ok_or(RuntimeError::Protocol)?;
        let snapshot = self
            .context
            .coordinator
            .snapshot(&registration.database)
            .map_err(sync_error)?;
        if known == Some(snapshot.fingerprint()) {
            return Ok(IoValue::Unchanged);
        }
        let mut file =
            NamedTempFile::new_in(&self.directory).map_err(|_| RuntimeError::Transport)?;
        snapshot
            .copy_ciphertext(&mut file)
            .map_err(cipher_ipc::storage)?;
        let spool = file.path().to_owned();
        self.spools.push(file);
        Ok(IoValue::Snapshot {
            spool,
            fingerprint: snapshot.fingerprint(),
            root: registration.root,
            working_copy: registration.working_copy,
        })
    }
}
pub(crate) fn canonical_path(path: &Path) -> Result<PathBuf, RuntimeError> {
    if path.try_exists().map_err(|_| RuntimeError::Transport)? {
        return path.canonicalize().map_err(|_| RuntimeError::Transport);
    }
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    Ok(parent
        .canonicalize()
        .map_err(|_| RuntimeError::Transport)?
        .join(path.file_name().ok_or(RuntimeError::Protocol)?))
}
