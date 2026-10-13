//! Real PUBLIC archives prove exact copying without a plaintext or author capability.
use super::*;
use crate::profile::{CredentialStore, ProfileError};
use std::{
    fs,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
};
use taypeer_core::EntryId;
use taypeer_services::{
    AttachmentImport, BinaryTarget, DatabaseService, EntryPatch, FieldUpdate, SessionToken,
    new_operation_id,
};
use zeroize::Zeroizing;

const PASSWORD: &[u8] = b"PUBLIC ciphertext export master";
type CredentialValues = BTreeMap<(String, String), Vec<u8>>;
#[derive(Default)]
struct Credentials {
    values: Mutex<CredentialValues>,
    deny_author: AtomicBool,
    deny_state: AtomicBool,
    author_reads: AtomicUsize,
}
impl CredentialStore for Credentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        if account == "author" {
            self.author_reads.fetch_add(1, Ordering::Relaxed);
            if self.deny_author.load(Ordering::Relaxed) {
                return Err(ProfileError::Credentials);
            }
        } else if self.deny_state.load(Ordering::Relaxed) {
            return Err(ProfileError::Invalid);
        }
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned()
            .map(Zeroizing::new))
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        self.values
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), bytes.to_vec());
        Ok(())
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    credentials: Arc<Credentials>,
    host: RuntimeHost,
    service: DatabaseService,
    session: SessionToken,
    copy: WorkingCopy,
    entry: EntryId,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let credentials = Arc::new(Credentials::default());
        let host = RuntimeHost::with_platform_credentials(
            &directory.path().join("profile"),
            crate::session::SessionController::new(Default::default()),
            credentials.clone(),
        )
        .unwrap();
        let path = directory.path().join("PUBLIC.taypeer");
        let mut service = DatabaseService::new();
        let session = host
            .open_local(
                &mut service,
                &path,
                PASSWORD,
                Some("PUBLIC encrypted export".into()),
            )
            .unwrap();
        let group = service
            .create_group(
                &session,
                "PUBLIC group".into(),
                None,
                &new_operation_id().unwrap(),
            )
            .unwrap()
            .value
            .id;
        let entry = service
            .create_entry(
                &session,
                group,
                EntryPatch {
                    title: FieldUpdate::Set("PUBLIC saved original".into()),
                    password: FieldUpdate::Set("PUBLIC entry password".into()),
                    ..Default::default()
                },
                &new_operation_id().unwrap(),
            )
            .unwrap()
            .value;
        let copy = host.register_working_copy(&path).unwrap();
        Self {
            directory,
            credentials,
            host,
            service,
            session,
            copy,
            entry,
        }
    }
    fn lock(&mut self) {
        self.service.lock(&self.session).unwrap();
        self.host.close(&self.session.database).unwrap();
    }
}

#[test]
fn ciphertext_export_preserves_history_blobs_inventory_and_does_not_include_local_drafts() {
    let mut fixture = Fixture::new();
    let blob: Vec<u8> = (0..200_000).map(|i| (i % 251) as u8).collect();
    fixture
        .service
        .start_edit_entry(&fixture.session, &fixture.entry)
        .unwrap();
    let draft = fixture
        .service
        .active_draft(&fixture.session)
        .unwrap()
        .value
        .unwrap();
    fixture
        .service
        .import_attachment_stream(
            &fixture.session,
            &draft.draft,
            blob.as_slice(),
            AttachmentImport {
                length: blob.len() as u64,
                name: "PUBLIC saved binary.bin".into(),
                replacement: None,
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    fixture
        .service
        .save_draft(&fixture.session, &new_operation_id().unwrap())
        .unwrap();
    let saved = fixture
        .service
        .view_entry(&fixture.session, &fixture.entry)
        .unwrap()
        .value;
    assert_eq!(
        fixture
            .service
            .history(&fixture.session, &fixture.entry)
            .unwrap()
            .value
            .len(),
        2
    );
    let inventory = fixture
        .host
        .coordinator()
        .snapshot(&fixture.copy.database)
        .unwrap()
        .metadata()
        .manifest
        .body
        .objects
        .clone();
    // An encrypted, unfinished local binary form must not leak into the portable copy.
    fixture
        .service
        .start_edit_entry(&fixture.session, &fixture.entry)
        .unwrap();
    let draft = fixture
        .service
        .active_draft(&fixture.session)
        .unwrap()
        .value
        .unwrap();
    fixture
        .service
        .import_attachment_stream(
            &fixture.session,
            &draft.draft,
            b"PUBLIC unsaved binary".as_slice(),
            AttachmentImport {
                length: b"PUBLIC unsaved binary".len() as u64,
                name: "PUBLIC local draft.bin".into(),
                replacement: None,
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let source = fs::read(&fixture.copy.path).unwrap();
    let working_copy = fixture
        .host
        .profile()
        .registration(&fixture.copy.path)
        .unwrap()
        .unwrap()
        .working_copy;
    let sidecar = fixture
        .copy
        .path
        .with_file_name(format!("PUBLIC.taypeer.{working_copy}.draft"));
    let draft_bytes = fs::read(&sidecar).unwrap();
    let catalog = fs::read(fixture.directory.path().join("profile/working-copies.json")).unwrap();
    let capture = fixture
        .host
        .capture_working_copy_export(&fixture.copy)
        .unwrap();
    let mut bytes = Vec::new();
    capture.copy_to(&mut bytes).unwrap();
    assert_eq!(bytes, source);
    assert_eq!(capture.identity().bytes, source.len() as u64);
    assert_eq!(fs::read(&fixture.copy.path).unwrap(), source);
    assert_eq!(fs::read(sidecar).unwrap(), draft_bytes);
    assert_eq!(
        fs::read(fixture.directory.path().join("profile/working-copies.json")).unwrap(),
        catalog
    );

    let destination = fixture.directory.path().join("PUBLIC external.taypeer");
    fs::write(&destination, bytes).unwrap();
    let snapshot = ArchiveSnapshot::open(&destination, Some(fixture.copy.root)).unwrap();
    assert_eq!(snapshot.fingerprint(), capture.identity().fingerprint);
    assert_eq!(snapshot.metadata().manifest.body.objects, inventory);
    let destination_host = RuntimeHost::with_platform_credentials(
        &fixture.directory.path().join("destination-profile"),
        crate::session::SessionController::new(Default::default()),
        Arc::new(Credentials::default()),
    )
    .unwrap();
    let mut reopened = DatabaseService::new();
    let session = destination_host
        .open_local(&mut reopened, &destination, PASSWORD, None)
        .unwrap();
    assert_eq!(
        reopened
            .history(&session, &fixture.entry)
            .unwrap()
            .value
            .len(),
        2
    );
    assert!(reopened.active_draft(&session).unwrap().value.is_none());
    assert_eq!(
        reopened
            .view_entry(&session, &fixture.entry)
            .unwrap()
            .value
            .attachments
            .len(),
        1
    );
    let mut exported_blob = Vec::new();
    reopened
        .export_binary_stream(
            &session,
            &BinaryTarget::Entry(fixture.entry.clone()),
            &saved.attachments[0].blob,
            &mut exported_blob,
        )
        .unwrap();
    assert_eq!(exported_blob, blob);
    reopened.lock(&session).unwrap();
    destination_host.close(&session.database).unwrap();
}

#[test]
fn later_save_and_host_closure_never_replace_the_captured_generation() {
    let mut fixture = Fixture::new();
    let before = fs::read(&fixture.copy.path).unwrap();
    let capture = fixture
        .host
        .capture_working_copy_export(&fixture.copy)
        .unwrap();
    fixture
        .service
        .update_entry(
            &fixture.session,
            &fixture.entry,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC newer generation".into()),
                ..Default::default()
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    assert_ne!(fs::read(&fixture.copy.path).unwrap(), before);
    let latest = fixture
        .host
        .capture_working_copy_export(&fixture.copy)
        .unwrap();
    assert_ne!(
        capture.identity().fingerprint,
        latest.identity().fingerprint
    );
    fixture.lock();
    drop(fixture.service);
    drop(fixture.host);
    let mut bytes = Vec::new();
    capture.copy_to(&mut bytes).unwrap();
    assert_eq!(bytes, before);
    let mut repeat = Vec::new();
    capture.copy_to(&mut repeat).unwrap();
    assert_eq!(repeat, before);
}

#[test]
fn locked_registered_copy_exports_without_author_or_writer_and_rollback_is_refused() {
    let mut fixture = Fixture::new();
    let prior = fs::read(&fixture.copy.path).unwrap();
    fixture
        .service
        .update_entry(
            &fixture.session,
            &fixture.entry,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC latest".into()),
                ..Default::default()
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let current = fs::read(&fixture.copy.path).unwrap();
    fixture.lock();
    fixture
        .credentials
        .deny_author
        .store(true, Ordering::Relaxed);
    let author_reads = fixture.credentials.author_reads.load(Ordering::Relaxed);
    let capture = fixture
        .host
        .capture_working_copy_export(&fixture.copy)
        .unwrap();
    let mut bytes = Vec::new();
    capture.copy_to(&mut bytes).unwrap();
    assert_eq!(bytes, current);
    assert_eq!(
        fixture.credentials.author_reads.load(Ordering::Relaxed),
        author_reads
    );
    assert!(fixture.host.context.copies.lock().unwrap().is_empty());
    assert!(
        fixture
            .host
            .coordinator()
            .snapshot(&fixture.copy.database)
            .is_err()
    );
    fs::write(&fixture.copy.path, prior).unwrap();
    assert!(matches!(
        fixture.host.capture_working_copy_export(&fixture.copy),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::Changed)
        ))
    ));
}

#[test]
fn exact_catalog_scope_and_protected_registration_failures_cannot_become_a_capture() {
    let fixture = Fixture::new();
    for invalid in [
        WorkingCopy {
            database: DatabaseId::new("PUBLIC foreign"),
            ..fixture.copy.clone()
        },
        WorkingCopy {
            root: Digest::of(b"PUBLIC wrong root"),
            ..fixture.copy.clone()
        },
        WorkingCopy {
            path: fixture.directory.path().join("PUBLIC unregistered.taypeer"),
            ..fixture.copy.clone()
        },
    ] {
        assert!(matches!(
            fixture.host.capture_working_copy_export(&invalid),
            Err(RuntimeError::Closed)
        ));
    }
    fixture
        .credentials
        .deny_state
        .store(true, Ordering::Relaxed);
    assert!(matches!(
        fixture.host.capture_working_copy_export(&fixture.copy),
        Err(RuntimeError::Profile(ProfileError::Invalid))
    ));
}
