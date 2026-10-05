//! Nonsecret catalog of durable working files. No document labels or credentials are stored here.
use super::*;
use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read};
use taypeer_core::OperationId;
use taypeer_storage::PublicationMode;

const VERSION: u16 = 1;
const MAX_CATALOG_BYTES: u64 = 1024 * 1024;

/// A verified working file. The catalog contains only public identities and local paths.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkingCopy {
    /// Logical database identity.
    pub database: DatabaseId,
    /// Pinned trust lineage; importing a file never grants device admission.
    pub root: Digest,
    /// Canonical path of the durable working file.
    pub path: PathBuf,
}

/// A durably relocated working file and the outcome of removing its previous ciphertext.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RelocatedWorkingCopy {
    /// Published catalog entry at the destination.
    pub copy: WorkingCopy,
    /// Previous ciphertext was retained when cleanup failed. It is no longer the working file.
    pub retained_source: Option<PathBuf>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CatalogFile {
    version: u16,
    copies: Vec<WorkingCopy>,
}

pub(super) struct Catalog {
    directory: PathBuf,
    copies: BTreeMap<DatabaseId, WorkingCopy>,
}
impl Catalog {
    pub fn load(profile: &Path) -> Result<Self, RuntimeError> {
        let directory = profile
            .canonicalize()
            .map_err(|_| RuntimeError::Transport)?;
        let path = directory.join("working-copies.json");
        let copies = match File::open(path) {
            Ok(file) => {
                let mut bytes = Vec::new();
                file.take(MAX_CATALOG_BYTES + 1)
                    .read_to_end(&mut bytes)
                    .map_err(|_| RuntimeError::Transport)?;
                if bytes.len() as u64 > MAX_CATALOG_BYTES {
                    return Err(RuntimeError::TooLarge);
                }
                let file: CatalogFile =
                    serde_json::from_slice(&bytes).map_err(|_| RuntimeError::Protocol)?;
                if file.version != VERSION {
                    return Err(RuntimeError::Protocol);
                }
                // A previous host may have lost the response after replacement. Confirm
                // durability before exposing that generation as the working catalog.
                File::open(directory.join("working-copies.json"))
                    .and_then(|file| file.sync_all())
                    .and_then(|()| File::open(&directory).and_then(|file| file.sync_all()))
                    .map_err(|_| cipher_ipc::storage(taypeer_storage::Error::CommitUncertain))?;
                let mut copies = BTreeMap::new();
                for copy in file.copies {
                    if !copy.path.is_absolute()
                        || copy.path.file_name().is_none()
                        || copies.insert(copy.database.clone(), copy).is_some()
                    {
                        return Err(RuntimeError::Protocol);
                    }
                }
                copies
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => BTreeMap::new(),
            Err(_) => return Err(RuntimeError::Transport),
        };
        Ok(Self { directory, copies })
    }

    fn working_directory(&self) -> Result<PathBuf, RuntimeError> {
        let directory = self.directory.join("working-copies");
        #[cfg(unix)]
        {
            use std::os::unix::fs::DirBuilderExt;
            std::fs::DirBuilder::new()
                .recursive(true)
                .mode(0o700)
                .create(&directory)
                .map_err(|_| RuntimeError::Transport)?;
        }
        #[cfg(not(unix))]
        std::fs::create_dir_all(&directory).map_err(|_| RuntimeError::Transport)?;
        directory
            .canonicalize()
            .map_err(|_| RuntimeError::Transport)
    }

    fn creation_path(&self, operation: &OperationId) -> Result<PathBuf, RuntimeError> {
        // Public identifiers are hashed rather than interpolated as filenames.
        let id = Digest::object(b"taypeer.working-copy.creation.v1", operation)
            .map_err(|_| RuntimeError::Protocol)?;
        Ok(self.working_directory()?.join(format!("{id}.taypeer")))
    }

    fn stage_external(&self, source: &Path) -> Result<WorkingCopy, RuntimeError> {
        let source = source.canonicalize().map_err(|_| RuntimeError::Transport)?;
        let snapshot = ArchiveSnapshot::open(&source, None).map_err(cipher_ipc::storage)?;
        let database = snapshot.chain().head().database.clone();
        let root = snapshot
            .chain()
            .root()
            .map_err(|_| RuntimeError::Protocol)?;
        if let Some(copy) = self.copies.get(&database) {
            if copy.root != root {
                return Err(RuntimeError::Protocol);
            }
            verify(copy)?;
            return Ok(copy.clone());
        }
        let id = Digest::object(b"taypeer.working-copy.import.v1", &database)
            .map_err(|_| RuntimeError::Protocol)?;
        let path = self.working_directory()?.join(format!("{id}.taypeer"));
        let copy = WorkingCopy {
            database,
            root,
            path,
        };
        if copy
            .path
            .try_exists()
            .map_err(|_| RuntimeError::Transport)?
        {
            // Reuse a complete file left by an interrupted catalog publication.
            verify(&copy)?;
            return Ok(copy);
        }
        let mut temp = NamedTempFile::new_in(self.working_directory()?)
            .map_err(|_| RuntimeError::Transport)?;
        let mut input = File::open(&source).map_err(|_| RuntimeError::Transport)?;
        std::io::copy(&mut input, &mut temp).map_err(|_| RuntimeError::Transport)?;
        let candidate =
            ArchiveSnapshot::open(temp.path(), Some(root)).map_err(cipher_ipc::storage)?;
        if candidate.fingerprint() != snapshot.fingerprint() {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
        }
        drop(candidate);
        taypeer_storage::publish_file(temp, &copy.path, PublicationMode::Create)
            .map_err(cipher_ipc::storage)?;
        Ok(copy)
    }

    fn record(&mut self, copy: WorkingCopy) -> Result<(), RuntimeError> {
        verify(&copy)?;
        if self.copies.get(&copy.database) == Some(&copy) {
            return Ok(());
        }
        if self
            .copies
            .get(&copy.database)
            .is_some_and(|prior| prior.root != copy.root)
        {
            return Err(RuntimeError::Protocol);
        }
        let mut candidate = self.copies.clone();
        candidate.insert(copy.database.clone(), copy);
        let bytes = serde_json::to_vec(&CatalogFile {
            version: VERSION,
            copies: candidate.values().cloned().collect(),
        })
        .map_err(|_| RuntimeError::Protocol)?;
        if bytes.len() as u64 > MAX_CATALOG_BYTES {
            return Err(RuntimeError::TooLarge);
        }
        let mut temp =
            NamedTempFile::new_in(&self.directory).map_err(|_| RuntimeError::Transport)?;
        use std::io::Write;
        temp.write_all(&bytes)
            .map_err(|_| RuntimeError::Transport)?;
        taypeer_storage::publish_file(
            temp,
            &self.directory.join("working-copies.json"),
            PublicationMode::Replace,
        )
        .map_err(cipher_ipc::storage)?;
        self.copies = candidate;
        Ok(())
    }
}

fn verify(copy: &WorkingCopy) -> Result<(), RuntimeError> {
    let snapshot =
        ArchiveSnapshot::open(&copy.path, Some(copy.root)).map_err(cipher_ipc::storage)?;
    if snapshot.chain().head().database != copy.database {
        return Err(RuntimeError::Protocol);
    }
    Ok(())
}

impl RuntimeHost {
    /// Read the nonsecret startup catalog without acquiring credentials or creating a
    /// profile. A missing profile is empty; malformed or unavailable data is an error.
    pub fn read_working_copies(profile: &Path) -> Result<Vec<WorkingCopy>, RuntimeError> {
        if !profile.try_exists().map_err(|_| RuntimeError::Transport)? {
            return Ok(Vec::new());
        }
        Ok(Catalog::load(profile)?.copies.into_values().collect())
    }

    /// List durable local files without revealing names, unlocking or acquiring author credentials.
    pub fn working_copies(&self) -> Result<Vec<WorkingCopy>, RuntimeError> {
        Ok(self
            .catalog
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .copies
            .values()
            .cloned()
            .collect())
    }

    /// Internal destination for an idempotent creation intent. Nothing enters the catalog yet.
    pub fn creation_path(&self, operation: &OperationId) -> Result<PathBuf, RuntimeError> {
        self.catalog
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .creation_path(operation)
    }

    /// Present an invitation using a host-selected internal destination. Interrupted
    /// requests resume their original destination and never create another working file.
    pub fn join_internal_cancellable(
        &self,
        executable: &Path,
        code: crate::InvitationCode,
        password: String,
        cancellation: &crate::NetworkCancellation,
    ) -> Result<crate::JoinProgress, RuntimeError> {
        let request = code.invitation.id().map_err(|_| RuntimeError::Protocol)?;
        if self.pending_join_summaries()?.contains_key(&request) {
            return self.resume_internal_join_cancellable(request, password, cancellation);
        }
        if self
            .working_copies()?
            .iter()
            .any(|copy| copy.database == code.invitation.database)
        {
            return Err(taypeer_services::ServiceError::OperationConflict.into());
        }
        let operation = OperationId::new(format!("join:{request}"));
        let path = self.creation_path(&operation)?;
        self.join_cancellable(executable, code, &path, password, cancellation)
    }

    /// Resume an internal receipt and publish its catalog entry only after the complete
    /// archive and native registration are durable. Approval is still explicit at the manager.
    pub fn resume_internal_join_cancellable(
        &self,
        request: Digest,
        password: String,
        cancellation: &crate::NetworkCancellation,
    ) -> Result<crate::JoinProgress, RuntimeError> {
        let path = self
            .pending_join_summaries()?
            .get(&request)
            .map(|pending| pending.path.clone());
        let progress = self.resume_join_cancellable(request, password, cancellation)?;
        if let crate::JoinProgress::Received(ref database) = progress {
            if let Some(path) = path {
                self.register_working_copy(&path)?;
            } else if !self
                .working_copies()?
                .iter()
                .any(|copy| &copy.database == database)
            {
                return Err(RuntimeError::Protocol);
            }
        }
        Ok(progress)
    }

    /// Copy a verified external archive internally, preserving its source. This is staging only:
    /// registration and authentication must succeed before `register_working_copy` publishes it.
    pub fn stage_external_copy(&self, source: &Path) -> Result<WorkingCopy, RuntimeError> {
        self.catalog
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .stage_external(source)
    }

    /// Publish an authenticated, registered working file after its durable creation or receipt.
    /// No new network admission is granted by this operation.
    pub fn register_working_copy(&self, path: &Path) -> Result<WorkingCopy, RuntimeError> {
        let path = canonical_path(path)?;
        let snapshot = ArchiveSnapshot::open(&path, None).map_err(cipher_ipc::storage)?;
        let database = snapshot.chain().head().database.clone();
        let root = snapshot
            .chain()
            .root()
            .map_err(|_| RuntimeError::Protocol)?;
        let context = self.context.for_database(&database)?;
        let registration = context
            .profile
            .registration(&path)?
            .ok_or(RuntimeError::Closed)?;
        if registration.database != database || registration.root != root {
            return Err(RuntimeError::Protocol);
        }
        let copy = WorkingCopy {
            database,
            root,
            path,
        };
        self.catalog
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .record(copy.clone())?;
        Ok(copy)
    }

    /// Authenticate an internal copy of an external file and publish it only after registration.
    pub fn open_external(
        &self,
        executable: &Path,
        source: &Path,
        password: String,
    ) -> Result<Worker, RuntimeError> {
        let copy = self.stage_external_copy(source)?;
        let worker = self.open(executable, &copy.path, password, None)?;
        if let Err(error) = self.register_working_copy(&copy.path) {
            worker.invalidate(crate::session::LockReason::Transport);
            return Err(error);
        }
        Ok(worker)
    }

    /// Create an internal working file. Repeating the creation operation reopens the existing
    /// file and verifies the original form rather than creating another database.
    pub fn create_internal(
        &self,
        executable: &Path,
        operation: &OperationId,
        password: String,
        form: taypeer_services::CreateDatabase,
    ) -> Result<Worker, RuntimeError> {
        let path = self.creation_path(operation)?;
        let exists = path.try_exists().map_err(|_| RuntimeError::Transport)?;
        let mut worker =
            self.open_configured(executable, &path, password, (!exists).then(|| form.clone()))?;
        if exists {
            let mut info = worker.request(&crate::Command::DatabaseInfo)?;
            let matches = info.get("name").and_then(serde_json::Value::as_str) == Some(&form.name)
                && info.get("description").and_then(serde_json::Value::as_str)
                    == form.description.as_deref();
            crate::erase_view(&mut info);
            let policy = worker.request(&crate::Command::DatabasePolicy)?;
            let actual: taypeer_core::DatabasePolicy =
                serde_json::from_value(policy).map_err(|_| RuntimeError::Protocol)?;
            if !matches || actual != form.policy {
                worker.invalidate(crate::session::LockReason::Transport);
                return Err(RuntimeError::Protocol);
            }
        }
        if let Err(error) = self.register_working_copy(&path) {
            worker.invalidate(crate::session::LockReason::Transport);
            return Err(error);
        }
        Ok(worker)
    }

    /// Explicitly relocate a catalog item after its worker and coordinator writer are closed.
    /// The destination, local draft and protected registration are durable before the catalog
    /// changes. Existing unrelated files are preserved. A cleanup failure returns the old path.
    pub fn relocate_working_copy(
        &self,
        database: &DatabaseId,
        destination: &Path,
    ) -> Result<RelocatedWorkingCopy, RuntimeError> {
        let mut catalog = self.catalog.lock().map_err(|_| RuntimeError::Transport)?;
        let source = catalog
            .copies
            .get(database)
            .cloned()
            .ok_or(RuntimeError::Closed)?;
        let destination = canonical_path(destination)?;
        if source.path == destination {
            return Ok(RelocatedWorkingCopy {
                copy: source,
                retained_source: None,
            });
        }
        let context = self.context.for_database(database)?;
        // Prevent a new attachment while the old path is being relocated.
        let copies = context.copies.lock().map_err(|_| RuntimeError::Transport)?;
        if copies
            .values()
            .any(|registration| &registration.database == database)
        {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Busy));
        }
        let registration = context
            .profile
            .registration(&source.path)?
            .ok_or(RuntimeError::Closed)?;
        let store = ArchiveStore::open(
            &source.path,
            Some(source.root),
            Some(context.profile.anchor(&registration)),
        )
        .map_err(cipher_ipc::storage)?;
        let snapshot = store.snapshot();
        let mut temp = NamedTempFile::new_in(destination.parent().ok_or(RuntimeError::Protocol)?)
            .map_err(|_| RuntimeError::Transport)?;
        std::io::copy(
            &mut File::open(&source.path).map_err(|_| RuntimeError::Transport)?,
            &mut temp,
        )
        .map_err(|_| RuntimeError::Transport)?;
        let candidate =
            ArchiveSnapshot::open(temp.path(), Some(source.root)).map_err(cipher_ipc::storage)?;
        if candidate.fingerprint() != snapshot.fingerprint() {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
        }
        drop(candidate);
        taypeer_storage::publish_file(temp, &destination, PublicationMode::Create)
            .map_err(cipher_ipc::storage)?;
        let copy = WorkingCopy {
            path: destination,
            ..source.clone()
        };
        context
            .profile
            .relocate_registration(&source.path, &copy.path)?;
        if let Some(draft) = taypeer_storage::read_local_draft(
            &source.path,
            registration.working_copy,
            snapshot.chain(),
        )
        .map_err(cipher_ipc::storage)?
        {
            taypeer_storage::save_local_draft(&copy.path, registration.working_copy, &draft)
                .map_err(cipher_ipc::storage)?;
        }
        catalog.record(copy.clone())?;
        drop(store);
        let removed = std::fs::remove_file(&source.path)
            .and_then(|()| {
                File::open(source.path.parent().ok_or(std::io::ErrorKind::NotFound)?)
                    .and_then(|file| file.sync_all())
            })
            .is_ok();
        let draft_removed = removed
            && taypeer_storage::discard_local_draft(&source.path, registration.working_copy)
                .is_ok();
        Ok(RelocatedWorkingCopy {
            copy,
            retained_source: (!draft_removed).then_some(source.path),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use taypeer_trust::{AuthorKey, Identity, ObjectKind};

    #[derive(Default)]
    struct PublicCredentials(Mutex<BTreeMap<(String, String), Vec<u8>>>);
    impl crate::profile::CredentialStore for PublicCredentials {
        fn get(
            &self,
            service: &str,
            account: &str,
        ) -> Result<Option<zeroize::Zeroizing<Vec<u8>>>, crate::profile::ProfileError> {
            Ok(self
                .0
                .lock()
                .unwrap()
                .get(&(service.to_owned(), account.to_owned()))
                .cloned()
                .map(zeroize::Zeroizing::new))
        }
        fn set(
            &self,
            service: &str,
            account: &str,
            bytes: &[u8],
        ) -> Result<(), crate::profile::ProfileError> {
            self.0
                .lock()
                .unwrap()
                .insert((service.to_owned(), account.to_owned()), bytes.to_vec());
            Ok(())
        }
    }

    fn owned_archive(
        host: &RuntimeHost,
        path: &Path,
    ) -> (WorkingCopy, taypeer_storage::EncryptedObject) {
        let author = AuthorKey::from_seed(&[41; 32]);
        let chain = ControlChain::genesis(
            DatabaseId::new("PUBLIC relocation"),
            Identity::new(author.public(), host.context.transport.public()).unwrap(),
            &author,
            Digest::of(b"PUBLIC policy"),
            taypeer_core::SchemaDescriptor::current(),
        )
        .unwrap();
        let (header, key) =
            taypeer_storage::create_epoch(b"PUBLIC relocation password", 500).unwrap();
        let object = |kind, bytes: &[u8]| {
            taypeer_storage::EncryptedObject::seal(
                &chain,
                &author,
                kind,
                &header,
                &key,
                bytes,
                bytes.len() as u64,
            )
            .unwrap()
        };
        let checkpoint = object(ObjectKind::Checkpoint, b"PUBLIC checkpoint");
        let baseline = object(ObjectKind::Baseline, b"PUBLIC baseline");
        host.context
            .create(
                path,
                ArchiveSeed {
                    controls: chain.records().to_vec(),
                    checkpoint: checkpoint.descriptor().digest,
                    baseline: baseline.descriptor().digest,
                    objects: vec![checkpoint, baseline],
                },
            )
            .unwrap();
        let copy = host.register_working_copy(path).unwrap();
        let registration = host.profile().registration(path).unwrap().unwrap();
        let draft = object(ObjectKind::LocalDraft, b"PUBLIC local collection");
        taypeer_storage::save_local_draft(path, registration.working_copy, &draft).unwrap();
        (copy, draft)
    }

    fn archive(path: &Path, database: &str) {
        let author = AuthorKey::from_seed(&[41; 32]);
        let transport = TransportKey::from_seed(&[42; 32]);
        let chain = ControlChain::genesis(
            DatabaseId::new(database),
            Identity::new(author.public(), transport.public()).unwrap(),
            &author,
            Digest::of(b"PUBLIC policy"),
            taypeer_core::SchemaDescriptor::current(),
        )
        .unwrap();
        let (header, key) = taypeer_storage::create_epoch(b"PUBLIC catalog password", 500).unwrap();
        let object = |kind, bytes: &[u8]| {
            taypeer_storage::EncryptedObject::seal(
                &chain,
                &author,
                kind,
                &header,
                &key,
                bytes,
                bytes.len() as u64,
            )
            .unwrap()
        };
        let checkpoint = object(ObjectKind::Checkpoint, b"PUBLIC checkpoint");
        let baseline = object(ObjectKind::Baseline, b"PUBLIC baseline");
        let seed = ArchiveSeed {
            controls: chain.records().to_vec(),
            checkpoint: checkpoint.descriptor().digest,
            baseline: baseline.descriptor().digest,
            objects: vec![checkpoint, baseline],
        };
        drop(seed.create(path, &transport, None).unwrap());
    }

    #[test]
    fn import_preserves_source_and_reuses_uncataloged_copy_after_failure() {
        let directory = tempfile::tempdir().unwrap();
        let profile = directory.path().join("profile");
        std::fs::create_dir(&profile).unwrap();
        let source = directory.path().join("PUBLIC external.taypeer");
        archive(&source, "PUBLIC ../../ filename escape");
        let before = std::fs::read(&source).unwrap();
        let mut catalog = Catalog::load(&profile).unwrap();
        let staged = catalog.stage_external(&source).unwrap();
        assert_ne!(staged.path, source);
        assert_eq!(
            staged.path.parent(),
            Some(
                profile
                    .canonicalize()
                    .unwrap()
                    .join("working-copies")
                    .as_path()
            )
        );
        assert_eq!(std::fs::read(&staged.path).unwrap(), before);
        assert!(catalog.copies.is_empty());
        // No catalog entry can be acknowledged when its publication fails.
        std::fs::create_dir(profile.join("working-copies.json")).unwrap();
        assert!(catalog.record(staged.clone()).is_err());
        assert!(catalog.copies.is_empty());
        assert_eq!(catalog.stage_external(&source).unwrap(), staged);
        std::fs::remove_dir(profile.join("working-copies.json")).unwrap();
        catalog.record(staged.clone()).unwrap();
        let reloaded = Catalog::load(&profile).unwrap();
        assert_eq!(reloaded.stage_external(&source).unwrap(), staged);
        assert_eq!(reloaded.copies.len(), 1);
        assert_eq!(std::fs::read(source).unwrap(), before);
    }

    #[test]
    fn duplicate_database_with_another_trust_root_is_preserved_and_rejected() {
        let directory = tempfile::tempdir().unwrap();
        let mut catalog = Catalog::load(directory.path()).unwrap();
        let first = directory.path().join("PUBLIC first.taypeer");
        let other = directory.path().join("PUBLIC other lineage.taypeer");
        archive(&first, "PUBLIC database");
        archive(&other, "PUBLIC database");
        let staged = catalog.stage_external(&first).unwrap();
        catalog.record(staged.clone()).unwrap();
        let bytes = std::fs::read(&staged.path).unwrap();
        assert!(matches!(
            catalog.stage_external(&other),
            Err(RuntimeError::Protocol)
        ));
        assert_eq!(std::fs::read(&staged.path).unwrap(), bytes);
        assert!(other.exists());
        assert_eq!(catalog.copies.len(), 1);
    }

    #[test]
    fn corrupt_and_future_catalogs_are_not_treated_as_empty() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("working-copies.json");
        for bytes in [b"{bad".as_slice(), br#"{"version":2,"copies":[]}"#] {
            std::fs::write(&path, bytes).unwrap();
            assert!(matches!(
                Catalog::load(directory.path()),
                Err(RuntimeError::Protocol)
            ));
            assert_eq!(std::fs::read(&path).unwrap(), bytes);
        }
    }

    #[test]
    fn creation_identifiers_cannot_select_paths_outside_the_profile() {
        let directory = tempfile::tempdir().unwrap();
        let catalog = Catalog::load(directory.path()).unwrap();
        let operation = OperationId::new("PUBLIC ../../ escape");
        let path = catalog.creation_path(&operation).unwrap();
        assert_eq!(catalog.creation_path(&operation).unwrap(), path);
        assert_eq!(
            path.parent(),
            Some(
                directory
                    .path()
                    .canonicalize()
                    .unwrap()
                    .join("working-copies")
                    .as_path()
            )
        );
        assert_eq!(path.extension().unwrap(), "taypeer");
    }

    #[test]
    fn profile_suffix_preserves_the_platforms_existing_gui_location() {
        let base = Path::new("/PUBLIC/data");
        let suffix = if cfg!(target_os = "macos") {
            "Taypeer"
        } else {
            "taypeer/profiles/default"
        };
        assert_eq!(profile_directory(base), base.join(suffix));
    }

    #[test]
    fn relocation_requires_closed_writers_and_preserves_draft_and_protected_identity() {
        let directory = tempfile::tempdir().unwrap();
        let host = RuntimeHost::with_platform_credentials(
            &directory.path().join("profile"),
            crate::session::SessionController::new(crate::session::SessionPolicy::default()),
            Arc::new(PublicCredentials::default()),
        )
        .unwrap();
        let source = canonical_path(&directory.path().join("PUBLIC source.taypeer")).unwrap();
        let (copy, draft) = owned_archive(&host, &source);
        let destination = directory.path().join("PUBLIC relocated.taypeer");
        assert!(matches!(
            host.relocate_working_copy(&copy.database, &destination),
            Err(RuntimeError::Service(
                taypeer_services::ServiceError::Storage(taypeer_storage::Error::Busy)
            ))
        ));
        host.close(&copy.database).unwrap();
        let registration = host.profile().registration(&source).unwrap().unwrap();
        let relocated = host
            .relocate_working_copy(&copy.database, &destination)
            .unwrap();
        assert!(relocated.retained_source.is_none());
        assert!(!source.exists());
        assert_eq!(host.working_copies().unwrap(), vec![relocated.copy.clone()]);
        let moved = host
            .profile()
            .registration(&relocated.copy.path)
            .unwrap()
            .unwrap();
        assert_eq!(moved.working_copy, registration.working_copy);
        let snapshot = ArchiveSnapshot::open(&relocated.copy.path, Some(copy.root)).unwrap();
        let restored = taypeer_storage::read_local_draft(
            &relocated.copy.path,
            moved.working_copy,
            snapshot.chain(),
        )
        .unwrap()
        .unwrap();
        assert_eq!(restored.descriptor(), draft.descriptor());
        assert!(
            taypeer_storage::read_local_draft(&source, registration.working_copy, snapshot.chain())
                .unwrap()
                .is_none()
        );
        assert_eq!(
            host.context
                .attach(&relocated.copy.path)
                .unwrap()
                .working_copy,
            registration.working_copy
        );
    }

    #[test]
    fn relocation_does_not_replace_an_existing_destination_or_publish_its_catalog_entry() {
        let directory = tempfile::tempdir().unwrap();
        let host = RuntimeHost::with_platform_credentials(
            &directory.path().join("profile"),
            crate::session::SessionController::new(crate::session::SessionPolicy::default()),
            Arc::new(PublicCredentials::default()),
        )
        .unwrap();
        let source = canonical_path(&directory.path().join("PUBLIC source.taypeer")).unwrap();
        let (copy, _) = owned_archive(&host, &source);
        host.close(&copy.database).unwrap();
        let destination = directory.path().join("PUBLIC unrelated");
        std::fs::write(&destination, b"PUBLIC unrelated bytes").unwrap();
        assert!(matches!(
            host.relocate_working_copy(&copy.database, &destination),
            Err(RuntimeError::Service(
                taypeer_services::ServiceError::Storage(taypeer_storage::Error::AlreadyExists)
            ))
        ));
        assert_eq!(
            std::fs::read(destination).unwrap(),
            b"PUBLIC unrelated bytes"
        );
        assert_eq!(host.working_copies().unwrap(), vec![copy]);
        assert!(source.exists());
    }
}
