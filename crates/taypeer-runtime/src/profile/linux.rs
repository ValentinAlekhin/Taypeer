//! Password-authenticated per-database credentials and independent host state.
#[cfg(all(test, target_os = "linux"))]
mod tests;

use super::*;
use std::{collections::BTreeMap, sync::Mutex};
use taypeer_storage::{LocalCredentialStore, LocalStateKey, LocalStateStore, ReadKey};

/// Private worker-to-host transport authority. Contains no author or database key.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportCapability {
    /// Authenticated database identity.
    pub database: DatabaseId,
    /// Pinned genesis control hash.
    pub root: Digest,
    /// Independent endpoint secret, usable only for ciphertext exchange.
    pub transport_seed: Zeroizing<[u8; 32]>,
    /// Independent local-state key; absent for a read/export-only copied archive.
    pub local_state_seed: Option<Zeroizing<[u8; 32]>>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalCredential {
    version: u16,
    database: DatabaseId,
    root: Digest,
    epoch: u64,
    author: Zeroizing<[u8; 32]>,
    local_state: Zeroizing<[u8; 32]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct JoinCredential {
    database: DatabaseId,
    root: Digest,
    author: Zeroizing<[u8; 32]>,
    transport: Zeroizing<[u8; 32]>,
    local_state: Zeroizing<[u8; 32]>,
}

#[derive(Default, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct State {
    values: BTreeMap<String, Zeroizing<Vec<u8>>>,
}

struct ProtectedState {
    database: DatabaseId,
    root: Digest,
    state: Mutex<StateOwner>,
}
struct StateOwner {
    store: Option<LocalStateStore>,
    key: LocalStateKey,
    values: State,
}
impl CredentialStore for ProtectedState {
    fn get(
        &self,
        _service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        // An author credential never enters this host-readable store.
        if account == "author" {
            return Err(ProfileError::Credentials);
        }
        self.state
            .lock()
            .map_err(|_| ProfileError::Io)
            .map(|state| state.values.values.get(account).cloned())
    }
    fn set(&self, _service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        if account == "author" || bytes.len() > 65536 {
            return Err(ProfileError::Invalid);
        }
        let mut state = self.state.lock().map_err(|_| ProfileError::Io)?;
        let mut candidate = State {
            values: state.values.values.clone(),
        };
        candidate
            .values
            .insert(account.to_owned(), Zeroizing::new(bytes.to_vec()));
        let clear = Zeroizing::new(
            serde_json::to_vec(&StateEnvelope {
                version: 1,
                database: self.database.clone(),
                root: self.root,
                values: candidate,
            })
            .map_err(|_| ProfileError::Invalid)?,
        );
        let StateOwner { store, key, .. } = &mut *state;
        if let Some(store) = store {
            store.save(key, &clear).map_err(storage)?;
        }
        let next: StateEnvelope =
            serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
        state.values = next.values;
        Ok(())
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StateEnvelope {
    version: u16,
    database: DatabaseId,
    root: Digest,
    values: State,
}

fn storage(error: taypeer_storage::Error) -> ProfileError {
    match error {
        taypeer_storage::Error::Io => ProfileError::Io,
        taypeer_storage::Error::CommitUncertain => ProfileError::CommitUncertain,
        _ => ProfileError::Invalid,
    }
}
fn private_directory(path: &Path) -> Result<(), ProfileError> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(path)
        .map_err(|_| ProfileError::Io)?;
    if let Some(parent) = path
        .parent()
        .filter(|parent| parent.file_name().is_some_and(|name| name == "databases"))
    {
        std::fs::set_permissions(parent, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| ProfileError::Io)?;
    }
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
        .map_err(|_| ProfileError::Io)
}
#[cfg(target_os = "linux")]
pub(super) fn acquire(directory: &Path) -> Result<ProfileLease, ProfileError> {
    private_directory(directory)?;
    let directory = directory.canonicalize().map_err(|_| ProfileError::Io)?;
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let lock = OpenOptions::new()
        .mode(0o600)
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(directory.join("host.lock"))
        .map_err(|_| ProfileError::Io)?;
    lock.set_permissions(std::fs::Permissions::from_mode(0o600))
        .map_err(|_| ProfileError::Io)?;
    taypeer_storage::try_lock_exclusive(&lock).map_err(ProfileError::from)?;
    let path = directory.join("profile.json");
    let profile = if path.try_exists().map_err(|_| ProfileError::Io)? {
        NativeProfile::load_with(&directory, Credentials::LazyLinux)?
    } else {
        // This nonsecret placeholder identifies the profile, never an enrolled endpoint.
        let placeholder = TransportKey::generate().map_err(|_| ProfileError::Credentials)?;
        let profile = NativeProfile {
            directory,
            credentials: Credentials::LazyLinux,
            public: PublicProfile {
                version: 1,
                id: random_id()?,
                transport: placeholder.public(),
            },
        };
        write_public(&path, &profile.public)?;
        profile
    };
    Ok(ProfileLease {
        profile,
        _lock: lock,
    })
}
impl NativeProfile {
    pub(crate) fn join_summaries(
        &self,
    ) -> Result<BTreeMap<Digest, crate::network::PendingJoinSummary>, ProfileError> {
        let path = self.directory.join("pending-joins.json");
        match File::open(path) {
            Ok(file) => {
                if file.metadata().map_err(|_| ProfileError::Io)?.len() > 65536 {
                    return Err(ProfileError::Invalid);
                }
                serde_json::from_reader(file).map_err(|_| ProfileError::Invalid)
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(BTreeMap::new()),
            Err(_) => Err(ProfileError::Io),
        }
    }
    fn save_join_summaries(
        &self,
        summaries: &BTreeMap<Digest, crate::network::PendingJoinSummary>,
    ) -> Result<(), ProfileError> {
        let mut file =
            tempfile::NamedTempFile::new_in(&self.directory).map_err(|_| ProfileError::Io)?;
        serde_json::to_writer(&mut file, summaries).map_err(|_| ProfileError::Invalid)?;
        file.as_file().sync_all().map_err(|_| ProfileError::Io)?;
        file.persist(self.directory.join("pending-joins.json"))
            .map_err(|_| ProfileError::Io)?;
        File::open(&self.directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| ProfileError::Io)
    }
    pub(crate) fn save_join_request(
        &self,
        request: Digest,
        pending: &crate::network::PendingJoin,
        password: &[u8],
    ) -> Result<(), ProfileError> {
        let directory = self.database_directory(&pending.invitation.database);
        private_directory(&directory)?;
        let path = directory.join("join.request");
        let clear = Zeroizing::new(serde_json::to_vec(pending).map_err(|_| ProfileError::Invalid)?);
        if path.try_exists().map_err(|_| ProfileError::Io)? {
            let mut store = taypeer_storage::FileStore::open(&path).map_err(storage)?;
            let (key, previous) = store.unlock(password).map_err(storage)?;
            let previous: crate::network::PendingJoin =
                serde_json::from_slice(&previous).map_err(|_| ProfileError::Invalid)?;
            if previous.proof != pending.proof || previous.path != pending.path {
                return Err(ProfileError::Invalid);
            }
            store.save(&key, &clear).map_err(storage)?;
        } else {
            taypeer_storage::FileStore::create(&path, password, &clear).map_err(storage)?;
        }
        let mut summaries = self.join_summaries()?;
        summaries.insert(
            request,
            crate::network::PendingJoinSummary {
                database: pending.invitation.database.clone(),
                path: pending.path.clone(),
            },
        );
        self.save_join_summaries(&summaries)
    }
    pub(crate) fn restore_join_request(
        &self,
        request: Digest,
        password: &[u8],
    ) -> Result<crate::network::PendingJoin, ProfileError> {
        let summaries = self.join_summaries()?;
        let summary = summaries.get(&request).ok_or(ProfileError::Invalid)?;
        let mut store = taypeer_storage::FileStore::open(
            &self
                .database_directory(&summary.database)
                .join("join.request"),
        )
        .map_err(storage)?;
        let (_, clear) = store.unlock(password).map_err(storage)?;
        let pending: crate::network::PendingJoin =
            serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
        if pending.invitation.id().map_err(|_| ProfileError::Invalid)? != request
            || pending.invitation.database != summary.database
            || pending.path != summary.path
        {
            return Err(ProfileError::Invalid);
        }
        Ok(pending)
    }
    pub(crate) fn finish_join_request(&self, request: Digest) -> Result<(), ProfileError> {
        let mut summaries = self.join_summaries()?;
        summaries.remove(&request);
        self.save_join_summaries(&summaries)
    }
    /// Retain an enrollment author only in password-encrypted staging until its epoch is known.
    pub(crate) fn prepare_join_credentials(
        &self,
        invitation: &taypeer_trust::Invitation,
        password: &[u8],
    ) -> Result<(AuthorKey, TransportCapability), ProfileError> {
        let directory = self.database_directory(&invitation.database);
        private_directory(&directory)?;
        let path = directory.join("join.credentials");
        let value = if path.try_exists().map_err(|_| ProfileError::Io)? {
            let mut store = taypeer_storage::FileStore::open(&path).map_err(storage)?;
            let (_, clear) = store.unlock(password).map_err(storage)?;
            let value: JoinCredential =
                serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
            if value.database != invitation.database || value.root != invitation.root {
                return Err(ProfileError::Invalid);
            }
            value
        } else {
            let author = AuthorKey::generate().map_err(|_| ProfileError::Credentials)?;
            let transport = TransportKey::generate().map_err(|_| ProfileError::Credentials)?;
            let local = LocalStateKey::generate().map_err(storage)?;
            let value = JoinCredential {
                database: invitation.database.clone(),
                root: invitation.root,
                author: author.secret_seed(),
                transport: transport.secret_seed(),
                local_state: Zeroizing::new(*local.secret_bytes()),
            };
            let clear =
                Zeroizing::new(serde_json::to_vec(&value).map_err(|_| ProfileError::Invalid)?);
            taypeer_storage::FileStore::create(&path, password, &clear).map_err(storage)?;
            value
        };
        let capability = TransportCapability {
            database: value.database,
            root: value.root,
            transport_seed: value.transport,
            local_state_seed: Some(value.local_state),
        };
        let (active, _) = self.activate(&capability)?;
        drop(active);
        Ok((AuthorKey::from_seed(&value.author), capability))
    }
    /// Bind password-encrypted enrollment staging to a fully verified received database epoch.
    pub(crate) fn bind_join_credentials(
        &self,
        snapshot: &ArchiveSnapshot,
        password: &[u8],
        key: &ReadKey,
    ) -> Result<(), ProfileError> {
        let database = &snapshot.chain().head().database;
        let directory = self.database_directory(database);
        let path = directory.join("join.credentials");
        if !path.try_exists().map_err(|_| ProfileError::Io)?
            || directory
                .join("credentials")
                .try_exists()
                .map_err(|_| ProfileError::Io)?
        {
            return Ok(());
        }
        let mut store = taypeer_storage::FileStore::open(&path).map_err(storage)?;
        let (_, clear) = store.unlock(password).map_err(storage)?;
        let value: JoinCredential =
            serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
        let author = AuthorKey::from_seed(&value.author);
        let transport = TransportKey::from_seed(&value.transport);
        let identity = Identity::new(author.public(), transport.public())
            .map_err(|_| ProfileError::Invalid)?;
        if value.database != *database
            || value.root != snapshot.chain().root().map_err(|_| ProfileError::Invalid)?
            || snapshot
                .chain()
                .head()
                .members
                .get(&identity.device)
                .is_none_or(|member| member.identity != identity)
        {
            return Err(ProfileError::Invalid);
        }
        let credential = LocalCredential {
            version: 1,
            database: value.database,
            root: value.root,
            epoch: snapshot.chain().head().epoch,
            author: value.author,
            local_state: value.local_state,
        };
        let clear =
            Zeroizing::new(serde_json::to_vec(&credential).map_err(|_| ProfileError::Invalid)?);
        LocalCredentialStore::create(&directory.join("credentials"), key, &clear)
            .map_err(storage)?;
        Ok(())
    }
    /// Prepare a new epoch-bound credential object without replacing the active one.
    pub(crate) fn prepare_credential_rotation(
        &self,
        database: &DatabaseId,
        old: &ReadKey,
        new: &ReadKey,
        epoch: u64,
    ) -> Result<(), taypeer_storage::Error> {
        let directory = self.database_directory(database);
        let current = LocalCredentialStore::open(&directory.join("credentials"))?;
        let clear = current.unlock(old)?;
        let mut value: LocalCredential =
            serde_json::from_slice(&clear).map_err(|_| taypeer_storage::Error::InvalidFile)?;
        if value.database != *database || value.epoch.checked_add(1) != Some(epoch) {
            return Err(taypeer_storage::Error::Changed);
        }
        value.epoch = epoch;
        let clear = Zeroizing::new(
            serde_json::to_vec(&value).map_err(|_| taypeer_storage::Error::InvalidFile)?,
        );
        let prepared = directory.join("credentials.prepared");
        if prepared
            .try_exists()
            .map_err(|_| taypeer_storage::Error::Io)?
        {
            // An earlier preparation is never silently replaced with a different intent.
            let mut prior = LocalCredentialStore::open(&prepared)?;
            prior.save(new, &clear)?;
        } else {
            LocalCredentialStore::create(&prepared, new, &clear)?;
        }
        Ok(())
    }
    /// Replace the active credentials only after the corresponding database commit.
    pub(crate) fn finalize_credential_rotation(
        &self,
        database: &DatabaseId,
    ) -> Result<(), taypeer_storage::Error> {
        let directory = self.database_directory(database);
        std::fs::rename(
            directory.join("credentials.prepared"),
            directory.join("credentials"),
        )
        .map_err(|_| taypeer_storage::Error::CommitUncertain)?;
        File::open(&directory)
            .and_then(|file| file.sync_all())
            .map_err(|_| taypeer_storage::Error::CommitUncertain)
    }
    /// Publish a recovered/learned epoch only after the host verified the working copy.
    pub(crate) fn finalize_authenticated_credentials(
        &self,
        database: &DatabaseId,
        root: Digest,
        key: &ReadKey,
        epoch: u64,
    ) -> Result<(), ProfileError> {
        let directory = self.database_directory(database);
        let prepared = directory.join("credentials.prepared");
        if !prepared.try_exists().map_err(|_| ProfileError::Io)? {
            return Ok(());
        }
        let store = LocalCredentialStore::open(&prepared).map_err(storage)?;
        let clear = match store.unlock(key) {
            Ok(clear) => clear,
            // A preparation for an epoch which never committed is preserved for retry.
            Err(taypeer_storage::Error::Authentication) => return Ok(()),
            Err(error) => return Err(storage(error)),
        };
        let value: LocalCredential =
            serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
        if value.version != 1
            || value.database != *database
            || value.root != root
            || value.epoch != epoch
        {
            return Err(ProfileError::Invalid);
        }
        drop(store);
        self.finalize_credential_rotation(database).map_err(storage)
    }
    /// Production Linux starts without acquiring any database transport secret.
    pub fn is_linux_lazy(&self) -> bool {
        #[cfg(target_os = "linux")]
        {
            matches!(self.credentials, Credentials::LazyLinux)
        }
        #[cfg(not(target_os = "linux"))]
        {
            false
        }
    }
    /// Whether this handle may persist registered state. Lazy roots have no active secrets.
    pub fn is_persistent(&self) -> bool {
        !self.is_linux_lazy() && !matches!(self.credentials, Credentials::Transient(_))
    }
    fn database_directory(&self, database: &DatabaseId) -> PathBuf {
        self.directory
            .join("databases")
            .join(Digest::of(database.as_str().as_bytes()).to_string())
    }
    /// Build a host context from authenticated worker authority, excluding author access.
    pub fn activate(
        &self,
        capability: &TransportCapability,
    ) -> Result<(NativeProfile, TransportKey), ProfileError> {
        let transport = TransportKey::from_seed(&capability.transport_seed);
        let persistent = capability.local_state_seed.is_some();
        let key = match &capability.local_state_seed {
            Some(seed) => LocalStateKey::from_secret(seed),
            None => LocalStateKey::generate().map_err(storage)?,
        };
        let path = self.database_directory(&capability.database).join("state");
        let mut initial = State::default();
        initial.values.insert(
            "transport".into(),
            Zeroizing::new(capability.transport_seed.to_vec()),
        );
        let (store, values) =
            if persistent {
                private_directory(path.parent().ok_or(ProfileError::Invalid)?)?;
                if path.try_exists().map_err(|_| ProfileError::Io)? {
                    let store = LocalStateStore::open(&path).map_err(storage)?;
                    let clear = store.unlock(&key).map_err(storage)?;
                    let state: StateEnvelope =
                        serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
                    if state.version != 1
                        || state.database != capability.database
                        || state.root != capability.root
                        || state.values.values.get("transport").is_none_or(|seed| {
                            seed.as_slice() != capability.transport_seed.as_slice()
                        })
                    {
                        return Err(ProfileError::Invalid);
                    }
                    (Some(store), state.values)
                } else {
                    let clear = Zeroizing::new(
                        serde_json::to_vec(&StateEnvelope {
                            version: 1,
                            database: capability.database.clone(),
                            root: capability.root,
                            values: initial,
                        })
                        .map_err(|_| ProfileError::Invalid)?,
                    );
                    let store = LocalStateStore::create(&path, &key, &clear).map_err(storage)?;
                    let state: StateEnvelope =
                        serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
                    (Some(store), state.values)
                }
            } else {
                (None, initial)
            };
        let store: Arc<dyn CredentialStore> = Arc::new(ProtectedState {
            database: capability.database.clone(),
            root: capability.root,
            state: Mutex::new(StateOwner { store, key, values }),
        });
        let credentials = if persistent {
            Credentials::Platform(store)
        } else {
            Credentials::Transient(store)
        };
        Ok((
            NativeProfile {
                directory: self.directory.clone(),
                public: PublicProfile {
                    transport: transport.public(),
                    ..self.public.clone()
                },
                credentials,
            },
            transport,
        ))
    }
    /// Enroll credentials only for explicit creation/join after authenticating the epoch.
    pub(crate) fn create_database_credentials(
        &self,
        database: &DatabaseId,
        root: Digest,
        epoch: u64,
        key: &ReadKey,
        author: &AuthorKey,
        transport: &TransportKey,
    ) -> Result<TransportCapability, ProfileError> {
        let directory = self.database_directory(database);
        private_directory(&directory)?;
        let local = LocalStateKey::generate().map_err(storage)?;
        let value = LocalCredential {
            version: 1,
            database: database.clone(),
            root,
            epoch,
            author: author.secret_seed(),
            local_state: Zeroizing::new(*local.secret_bytes()),
        };
        let clear = Zeroizing::new(serde_json::to_vec(&value).map_err(|_| ProfileError::Invalid)?);
        let store = LocalCredentialStore::create(&directory.join("credentials"), key, &clear)
            .map_err(storage)?;
        drop(store);
        let capability = TransportCapability {
            database: database.clone(),
            root,
            transport_seed: transport.secret_seed(),
            local_state_seed: Some(Zeroizing::new(*local.secret_bytes())),
        };
        let (active, _) = self.activate(&capability)?;
        drop(active);
        Ok(capability)
    }
    /// Authenticate local author credentials; absence grants only read/export access.
    pub(crate) fn database_credentials(
        &self,
        database: &DatabaseId,
        root: Digest,
        epoch: u64,
        key: &ReadKey,
        history: &[(u64, ReadKey)],
    ) -> Result<(Option<AuthorKey>, TransportCapability), ProfileError> {
        let directory = self.database_directory(database);
        let path = directory.join("credentials");
        if !path.try_exists().map_err(|_| ProfileError::Io)? {
            let transport = TransportKey::generate().map_err(|_| ProfileError::Credentials)?;
            return Ok((
                None,
                TransportCapability {
                    database: database.clone(),
                    root,
                    transport_seed: transport.secret_seed(),
                    local_state_seed: None,
                },
            ));
        }
        let store = LocalCredentialStore::open(&path).map_err(storage)?;
        let mut clear = store.unlock(key);
        if clear.is_err() {
            let prepared = directory.join("credentials.prepared");
            if prepared.try_exists().map_err(|_| ProfileError::Io)? {
                let prepared_store = LocalCredentialStore::open(&prepared).map_err(storage)?;
                clear = prepared_store.unlock(key);
            }
        }
        let mut historic = None;
        if clear.is_err() {
            for (old_epoch, old_key) in history {
                if *old_epoch == epoch {
                    continue;
                }
                if let Ok(candidate) = store.unlock(old_key) {
                    clear = Ok(candidate);
                    historic = Some(*old_epoch);
                    break;
                }
            }
        }
        let clear = clear.map_err(storage)?;
        let mut value: LocalCredential =
            serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
        if value.version != 1
            || value.database != *database
            || value.root != root
            || value.epoch != historic.unwrap_or(epoch)
        {
            return Err(ProfileError::Invalid);
        }
        if historic.is_some() {
            value.epoch = epoch;
            let clear =
                Zeroizing::new(serde_json::to_vec(&value).map_err(|_| ProfileError::Invalid)?);
            let prepared = directory.join("credentials.prepared");
            if prepared.try_exists().map_err(|_| ProfileError::Io)? {
                LocalCredentialStore::open(&prepared)
                    .map_err(storage)?
                    .save(key, &clear)
                    .map_err(storage)?;
            } else {
                LocalCredentialStore::create(&prepared, key, &clear).map_err(storage)?;
            }
        }
        drop(store);
        let local = LocalStateKey::from_secret(&value.local_state);
        let clear = LocalStateStore::read(&directory.join("state"), &local).map_err(storage)?;
        let state: StateEnvelope =
            serde_json::from_slice(&clear).map_err(|_| ProfileError::Invalid)?;
        if state.version != 1 || state.database != *database || state.root != root {
            return Err(ProfileError::Invalid);
        }
        let seed: &[u8; 32] = state
            .values
            .values
            .get("transport")
            .ok_or(ProfileError::Invalid)?
            .as_slice()
            .try_into()
            .map_err(|_| ProfileError::Invalid)?;
        Ok((
            Some(AuthorKey::from_seed(&value.author)),
            TransportCapability {
                database: database.clone(),
                root,
                transport_seed: Zeroizing::new(*seed),
                local_state_seed: Some(value.local_state),
            },
        ))
    }
}
