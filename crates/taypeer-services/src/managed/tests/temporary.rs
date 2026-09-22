use super::*;
use std::{
    io,
    path::Path,
    sync::atomic::{AtomicBool, Ordering},
};
use taypeer_storage::{CiphertextFile, TemporaryFileProvider, TemporaryStorage};

struct Allocator {
    directory: tempfile::TempDir,
    denied: AtomicBool,
}
impl TemporaryFileProvider for Allocator {
    fn create(&self) -> io::Result<CiphertextFile> {
        if self.denied.load(Ordering::SeqCst) {
            return Err(io::ErrorKind::PermissionDenied.into());
        }
        tempfile::tempfile_in(self.directory.path()).map(CiphertextFile::from)
    }
}
struct Port {
    inner: CoordinatorPersistence,
    temporary: TemporaryStorage,
}
impl CipherPersistence for Port {
    fn snapshot(&self) -> Result<ArchiveSnapshot, StorageError> {
        Ok(self
            .inner
            .snapshot()?
            .with_temporary_storage(self.temporary.clone()))
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, StorageError> {
        Ok(self
            .inner
            .commit(request)?
            .with_temporary_storage(self.temporary.clone()))
    }
    fn working_copy(&self) -> Digest {
        self.inner.working_copy()
    }
    fn path(&self) -> &Path {
        self.inner.path()
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), StorageError> {
        self.inner.save_draft(object)
    }
    fn load_draft(&self, chain: &ControlChain) -> Result<Option<EncryptedObject>, StorageError> {
        self.inner.load_draft(chain).map(|object| {
            object.map(|object| object.with_temporary_storage(self.temporary.clone()))
        })
    }
    fn discard_draft(&self) -> Result<(), StorageError> {
        self.inner.discard_draft()
    }
}

#[test]
fn explicit_staging_failure_preserves_saved_file_and_retryable_draft() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(33);
    let path = directory.path().join("PUBLIC.taypeer");
    let allocator = Arc::new(Allocator {
        directory: tempfile::tempdir().unwrap(),
        denied: AtomicBool::new(false),
    });
    let temporary = TemporaryStorage::new(allocator.clone());
    let seed = DatabaseService::prepare_managed_form_in(
        crate::CreateDatabase {
            name: "PUBLIC explicit staging".into(),
            description: None,
            policy: DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap(),
        },
        PASSWORD,
        &profile.author(),
        profile.identity(),
        1,
        temporary.clone(),
    )
    .unwrap();
    let store = seed.create(&path, &profile.transport, None).unwrap();
    let database = store.snapshot().chain().head().database.clone();
    profile.coordinator.register(store).unwrap();
    let make_port = || Port {
        inner: CoordinatorPersistence::new(
            Arc::clone(&profile.coordinator),
            database.clone(),
            path.clone(),
            Digest::of(&profile.seed),
        )
        .unwrap(),
        temporary: temporary.clone(),
    };
    let mut service = DatabaseService::new();
    let session = service
        .open_managed(Box::new(make_port()), PASSWORD, || {
            Ok(Some(profile.author()))
        })
        .unwrap();
    let group = service
        .create_group(
            &session,
            "PUBLIC group".into(),
            None,
            &crate::new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    service.start_create_entry(&session, group).unwrap();
    service
        .patch_draft(
            &session,
            crate::EntryPatch {
                title: crate::FieldUpdate::Set("PUBLIC retained candidate".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let operation = crate::new_operation_id().unwrap();
    let before = std::fs::read(&path).unwrap();
    allocator.denied.store(true, Ordering::SeqCst);
    assert!(matches!(
        service.save_draft(&session, &operation),
        Err(ServiceError::Storage(StorageError::Io))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(service.draft(&session).unwrap().value.unwrap().dirty);
    allocator.denied.store(false, Ordering::SeqCst);
    service.save_draft(&session, &operation).unwrap();
    service.lock(&session).unwrap();
    drop(service);
    let mut reopened = DatabaseService::new();
    let session = reopened
        .open_managed(Box::new(make_port()), PASSWORD, || {
            Ok(Some(profile.author()))
        })
        .unwrap();
    assert_eq!(reopened.entries(&session, None, "").unwrap().value.len(), 1);
}
