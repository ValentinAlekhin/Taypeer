//! Password-authenticated per-database credentials and independent host state.
#[cfg(test)]
mod tests;

use super::*;
use std::{collections::BTreeMap, sync::Mutex};
use taypeer_storage::{
    LocalCredential, LocalCredentialScope, LocalCredentialStore, LocalStateKey, LocalStateStore,
    ReadKey,
};

/// Private worker-to-host transport authority. Contains no author or database key.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct TransportCapability {
    /// Authenticated database identity.
    pub database: DatabaseId,
    /// Pinned genesis control hash.
    pub root: Digest,
    /// Explicit local enrollment permits transport. Durable read-only bookkeeping
    /// never starts an endpoint or supplies an author.
    pub enrolled: bool,
    /// Independent endpoint secret, usable only for ciphertext exchange.
    pub transport_seed: Zeroizing<[u8; 32]>,
    /// Independent local-state key; absent for a read/export-only copied archive.
    pub local_state_seed: Option<Zeroizing<[u8; 32]>>,
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
fn create_private_directory(path: &Path) -> Result<(), ProfileError> {
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
fn private_directory(path: &Path, boundary: &Path) -> Result<(), ProfileError> {
    private_directory_with_sync(path, boundary, |directory| {
        File::open(directory)?.sync_all()
    })
}
fn private_directory_with_sync(
    path: &Path,
    boundary: &Path,
    mut sync: impl FnMut(&Path) -> std::io::Result<()>,
) -> Result<(), ProfileError> {
    let boundary = boundary.canonicalize().map_err(|_| ProfileError::Io)?;
    let path = std::path::absolute(path).map_err(|_| ProfileError::Io)?;
    let existing = path
        .ancestors()
        .find(|parent| parent.is_dir())
        .ok_or(ProfileError::Invalid)?;
    if !existing
        .canonicalize()
        .map_err(|_| ProfileError::Io)?
        .starts_with(&boundary)
    {
        return Err(ProfileError::Invalid);
    }
    create_private_directory(&path)?;
    let path = path.canonicalize().map_err(|_| ProfileError::Io)?;
    if !path.starts_with(&boundary) {
        return Err(ProfileError::Invalid);
    }
    // Visible directories may remain from a failed earlier barrier. Confirm the
    // whole owned ancestry again, including each parent's entry for its child.
    for directory in path.ancestors() {
        sync(directory).map_err(|_| ProfileError::CommitUncertain)?;
        if directory == boundary {
            return Ok(());
        }
    }
    Err(ProfileError::Invalid)
}
pub(super) fn acquire(directory: &Path) -> Result<ProfileLease, ProfileError> {
    // The platform supplies the profile base; its initial provisioning durability
    // needs native acceptance. Never traverse sandbox-inaccessible OS ancestors.
    create_private_directory(directory)?;
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
        NativeProfile::load_with(&directory, Credentials::Portable)?
    } else {
        // This nonsecret placeholder identifies the profile, never an enrolled endpoint.
        let placeholder = TransportKey::generate().map_err(|_| ProfileError::Credentials)?;
        let profile = NativeProfile {
            directory,
            credentials: Credentials::Portable,
            scope: None,
            enrolled: false,
            capability: None,
            public: PublicProfile {
                version: 2,
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
    /// Read bounded ciphertext for an isolated worker. No key is acquired here.
    pub fn read_credential(
        &self,
        scope: &LocalCredentialScope,
        prepared: bool,
    ) -> Result<Option<Vec<u8>>, ProfileError> {
        let path = self.credential_path(scope, prepared);
        match File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(128 * 1024 + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| ProfileError::Io)?;
                if bytes.is_empty() || bytes.len() > 128 * 1024 {
                    return Err(ProfileError::Invalid);
                }
                Ok(Some(bytes))
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(_) => Err(ProfileError::Io),
        }
    }

    /// Durably publish worker-encrypted credentials; existing active enrollment is
    /// never replaced by a different creation request.
    pub fn publish_credential(
        &self,
        scope: &LocalCredentialScope,
        ciphertext: &[u8],
        prepared: bool,
    ) -> Result<(), ProfileError> {
        if ciphertext.is_empty() || ciphertext.len() > 128 * 1024 {
            return Err(ProfileError::Invalid);
        }
        let path = self.credential_path(scope, prepared);
        let directory = path.parent().ok_or(ProfileError::Invalid)?;
        private_directory(directory, &self.directory)?;
        let previous = self.read_credential(scope, prepared)?;
        if !prepared && previous.as_deref().is_some_and(|bytes| bytes != ciphertext) {
            return Err(ProfileError::Invalid);
        }
        let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| ProfileError::Io)?;
        file.write_all(ciphertext).map_err(|_| ProfileError::Io)?;
        let mode = if previous.is_some() {
            taypeer_storage::PublicationMode::Replace
        } else {
            taypeer_storage::PublicationMode::Create
        };
        taypeer_storage::publish_file(file, &path, mode).map_err(storage)
    }

    /// Complete a worker-verified epoch transition. Callers authenticate the
    /// resulting record before acknowledging success after a lost response.
    pub fn finalize_credential(&self, scope: &LocalCredentialScope) -> Result<(), ProfileError> {
        let prepared = self.credential_path(scope, true);
        if prepared.try_exists().map_err(|_| ProfileError::Io)? {
            std::fs::rename(prepared, self.credential_path(scope, false))
                .map_err(|_| ProfileError::CommitUncertain)?;
        } else if self.read_credential(scope, false)?.is_none() {
            return Err(ProfileError::Invalid);
        }
        self.confirm_active_credential(scope)
    }

    /// Confirm an authenticated active record without promoting unrelated staging.
    pub fn confirm_active_credential(
        &self,
        scope: &LocalCredentialScope,
    ) -> Result<(), ProfileError> {
        let directory = self.database_directory(&scope.database, scope.root);
        File::open(self.credential_path(scope, false))
            .and_then(|file| file.sync_all())
            .and_then(|()| File::open(directory).and_then(|file| file.sync_all()))
            .map_err(|_| ProfileError::CommitUncertain)
    }

    fn credential_path(&self, scope: &LocalCredentialScope, prepared: bool) -> PathBuf {
        self.database_directory(&scope.database, scope.root)
            .join(if prepared {
                "credentials.prepared"
            } else {
                "credentials"
            })
    }

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
        let directory = self.directory.join("joins").join(request.to_string());
        private_directory(&directory, &self.directory)?;
        let path = directory.join("request");
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
                .directory
                .join("joins")
                .join(request.to_string())
                .join("request"),
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
        let directory = self.database_directory(&invitation.database, invitation.root);
        private_directory(&directory, &self.directory)?;
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
            enrolled: true,
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
        let directory = self.database_directory(
            database,
            snapshot.chain().root().map_err(|_| ProfileError::Invalid)?,
        );
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
        let credential = LocalCredential::new(
            LocalCredentialScope {
                database: value.database,
                root: value.root,
                epoch: snapshot.chain().head().epoch,
            },
            &author,
            &transport,
            &LocalStateKey::from_secret(&value.local_state),
        );
        LocalCredentialStore::create_record(&directory.join("credentials"), key, &credential)
            .map_err(storage)?;
        Ok(())
    }
    /// Prepare a new epoch-bound credential object without replacing the active one.
    pub(crate) fn prepare_credential_rotation(
        &self,
        database: &DatabaseId,
        root: Digest,
        old: &ReadKey,
        new: &ReadKey,
        epoch: u64,
    ) -> Result<(), taypeer_storage::Error> {
        let directory = self.database_directory(database, root);
        let current = LocalCredentialStore::open(&directory.join("credentials"))?;
        let scope = LocalCredentialScope {
            database: database.clone(),
            root,
            epoch: epoch
                .checked_sub(1)
                .ok_or(taypeer_storage::Error::Changed)?,
        };
        let mut value = current.unlock_record(old, &scope)?;
        value.set_epoch(epoch);
        let prepared = directory.join("credentials.prepared");
        if prepared
            .try_exists()
            .map_err(|_| taypeer_storage::Error::Io)?
        {
            // An earlier preparation is never silently replaced with a different intent.
            let mut prior = LocalCredentialStore::open(&prepared)?;
            prior.save_record(new, &value)?;
        } else {
            LocalCredentialStore::create_record(&prepared, new, &value)?;
        }
        Ok(())
    }
    /// Replace the active credentials only after the corresponding database commit.
    pub(crate) fn finalize_credential_rotation(
        &self,
        database: &DatabaseId,
        root: Digest,
        key: &ReadKey,
        epoch: u64,
    ) -> Result<(), taypeer_storage::Error> {
        let directory = self.database_directory(database, root);
        let scope = LocalCredentialScope {
            database: database.clone(),
            root,
            epoch,
        };
        let prepared = directory.join("credentials.prepared");
        let promote = if prepared.try_exists()? {
            LocalCredentialStore::open(&prepared)?
                .unlock_record(key, &scope)
                .is_ok()
        } else {
            false
        };
        if promote {
            self.finalize_credential(&scope)
        } else {
            LocalCredentialStore::open(&directory.join("credentials"))?
                .unlock_record(key, &scope)?;
            self.confirm_active_credential(&scope)
        }
        .map_err(|error| match error {
            ProfileError::CommitUncertain => taypeer_storage::Error::CommitUncertain,
            ProfileError::Io => taypeer_storage::Error::Io,
            _ => taypeer_storage::Error::InvalidFile,
        })
    }
    /// Publish a recovered/learned epoch only after the host verified the working copy.
    pub(crate) fn finalize_authenticated_credentials(
        &self,
        database: &DatabaseId,
        root: Digest,
        key: &ReadKey,
        epoch: u64,
    ) -> Result<(), ProfileError> {
        let directory = self.database_directory(database, root);
        let prepared = directory.join("credentials.prepared");
        if !prepared.try_exists().map_err(|_| ProfileError::Io)? {
            return Ok(());
        }
        let store = LocalCredentialStore::open(&prepared).map_err(storage)?;
        let scope = LocalCredentialScope {
            database: database.clone(),
            root,
            epoch,
        };
        match store.unlock_record(key, &scope) {
            Ok(_) => {}
            // A preparation for an epoch which never committed is preserved for retry.
            Err(taypeer_storage::Error::Authentication) => return Ok(()),
            Err(error) => return Err(storage(error)),
        }
        drop(store);
        self.finalize_credential_rotation(database, root, key, epoch)
            .map_err(storage)
    }
    /// A dormant portable profile starts without any database transport secret.
    pub fn is_portable(&self) -> bool {
        matches!(self.credentials, Credentials::Portable)
    }
    /// Whether this handle may persist registered state. Lazy roots have no active secrets.
    pub fn is_persistent(&self) -> bool {
        !self.is_portable() && !matches!(self.credentials, Credentials::Transient(_))
    }
    pub(crate) fn database_directory(&self, database: &DatabaseId, root: Digest) -> PathBuf {
        self.directory
            .join("databases")
            .join(Digest::of(database.as_str().as_bytes()).to_string())
            .join(root.to_string())
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
        let path = self
            .database_directory(&capability.database, capability.root)
            .join("state");
        let mut initial = State::default();
        initial.values.insert(
            "transport".into(),
            Zeroizing::new(capability.transport_seed.to_vec()),
        );
        let (store, values) =
            if persistent {
                private_directory(path.parent().ok_or(ProfileError::Invalid)?, &self.directory)?;
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
                scope: Some((capability.database.clone(), capability.root)),
                enrolled: capability.enrolled,
                capability: Some(Arc::new(TransportCapability {
                    database: capability.database.clone(),
                    root: capability.root,
                    enrolled: capability.enrolled,
                    transport_seed: Zeroizing::new(*capability.transport_seed),
                    local_state_seed: capability
                        .local_state_seed
                        .as_ref()
                        .map(|seed| Zeroizing::new(**seed)),
                })),
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
        let directory = self.database_directory(database, root);
        private_directory(&directory, &self.directory)?;
        let local = LocalStateKey::generate().map_err(storage)?;
        let value = LocalCredential::new(
            LocalCredentialScope {
                database: database.clone(),
                root,
                epoch,
            },
            author,
            transport,
            &local,
        );
        let store =
            LocalCredentialStore::create_record(&directory.join("credentials"), key, &value)
                .map_err(storage)?;
        drop(store);
        let capability = TransportCapability {
            database: database.clone(),
            root,
            enrolled: true,
            transport_seed: transport.secret_seed(),
            local_state_seed: Some(Zeroizing::new(*local.secret_bytes())),
        };
        let (active, _) = self.activate(&capability)?;
        let identity = Identity::new(author.public(), transport.public())
            .map_err(|_| ProfileError::Invalid)?;
        active.credentials.set(
            &active.service(),
            "identity",
            &serde_json::to_vec(&identity).map_err(|_| ProfileError::Invalid)?,
        )?;
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
        let directory = self.database_directory(database, root);
        let path = directory.join("credentials");
        if !path.try_exists().map_err(|_| ProfileError::Io)? {
            private_directory(&directory, &self.directory)?;
            let scope = LocalCredentialScope {
                database: database.clone(),
                root,
                epoch,
            };
            let prepared = directory.join("credentials.prepared");
            let value = if prepared.try_exists().map_err(|_| ProfileError::Io)? {
                let store = LocalCredentialStore::open(&prepared).map_err(storage)?;
                let value = store.unlock_record(key, &scope).map_err(storage)?;
                if value.author().is_some() {
                    return Err(ProfileError::Invalid);
                }
                value
            } else {
                let transport = TransportKey::generate().map_err(|_| ProfileError::Credentials)?;
                let local = LocalStateKey::generate().map_err(storage)?;
                let value = LocalCredential::read_only(scope.clone(), &transport, &local);
                LocalCredentialStore::create_record(&prepared, key, &value).map_err(storage)?;
                value
            };
            let transport = value.transport();
            let local = value.local_state_key();
            let capability = TransportCapability {
                database: database.clone(),
                root,
                enrolled: false,
                transport_seed: transport.secret_seed(),
                local_state_seed: Some(Zeroizing::new(*local.secret_bytes())),
            };
            // Only local bookkeeping is enrolled; this transport is not admitted
            // by any signed control chain and never starts an endpoint.
            let (active, _) = self.activate(&capability)?;
            drop(active);
            self.finalize_credential(&scope)?;
            return Ok((None, capability));
        }
        let store = LocalCredentialStore::open(&path).map_err(storage)?;
        let scope = LocalCredentialScope {
            database: database.clone(),
            root,
            epoch,
        };
        let mut value = store.unlock_record(key, &scope);
        if value.is_err() {
            let prepared = directory.join("credentials.prepared");
            if prepared.try_exists().map_err(|_| ProfileError::Io)? {
                let prepared_store = LocalCredentialStore::open(&prepared).map_err(storage)?;
                value = prepared_store.unlock_record(key, &scope);
            }
        }
        let mut historic = None;
        if value.is_err() {
            for (old_epoch, old_key) in history {
                if *old_epoch == epoch {
                    continue;
                }
                let old_scope = LocalCredentialScope {
                    epoch: *old_epoch,
                    ..scope.clone()
                };
                if let Ok(candidate) = store.unlock_record(old_key, &old_scope) {
                    value = Ok(candidate);
                    historic = Some(*old_epoch);
                    break;
                }
            }
        }
        let mut value = value.map_err(storage)?;
        if historic.is_some() {
            value.set_epoch(epoch);
            let prepared = directory.join("credentials.prepared");
            if prepared.try_exists().map_err(|_| ProfileError::Io)? {
                LocalCredentialStore::open(&prepared)
                    .map_err(storage)?
                    .save_record(key, &value)
                    .map_err(storage)?;
            } else {
                LocalCredentialStore::create_record(&prepared, key, &value).map_err(storage)?;
            }
        }
        drop(store);
        let local = value.local_state_key();
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
        if *seed != *value.transport().secret_seed() {
            return Err(ProfileError::Invalid);
        }
        let author = value.author();
        let enrolled = author.is_some();
        Ok((
            author,
            TransportCapability {
                database: database.clone(),
                root,
                enrolled,
                transport_seed: Zeroizing::new(*seed),
                local_state_seed: Some(Zeroizing::new(*local.secret_bytes())),
            },
        ))
    }
}
