//! A hash-bound PUBLIC Android corpus, never an arbitrary user archive or writable profile.
use std::{
    collections::BTreeSet,
    fs::File,
    io::Read,
    path::Path,
    sync::{
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    time::Instant,
};
use taypeer_core::{IconRef, IconSource};
use taypeer_services::{BinaryTarget, DatabaseService, EntryView};
use taypeer_storage::{
    ArchiveSnapshot, CipherPersistence, EncryptedObject, Error, PreparedCommit, TemporaryStorage,
};
use taypeer_trust::{ControlChain, Digest};

const CAPTURE: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/../../tests/fixtures/android-eager-icons/seventy-image-entries.taypeer"
);
const BYTES: u64 = 4_567_715;
const SHA256: &str = "a965939a4ad83a973e93655d5b9ab435fd50c37c1acf08bfa6a4073985a3ea6d";
const PASSWORD: &[u8] = b"PUBLIC image list master";
const SVG: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="32" height="16"><rect width="32" height="16" fill="#14dc28"/></svg>"##;

/// Only the already-bound immutable ciphertext is visible to the service.
/// No author, registration, sidecar, source-file or protected-profile writer exists here.
struct ReadOnlyCapture {
    snapshot: ArchiveSnapshot,
    writes: Arc<AtomicUsize>,
}
impl CipherPersistence for ReadOnlyCapture {
    fn snapshot(&self) -> Result<ArchiveSnapshot, Error> {
        Ok(self.snapshot.clone())
    }
    fn commit(&self, _: PreparedCommit) -> Result<ArchiveSnapshot, Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Err(Error::Io)
    }
    fn working_copy(&self) -> Digest {
        Digest::of(b"PUBLIC read-only captured Android eager fixture")
    }
    fn save_draft(&self, _: &EncryptedObject) -> Result<(), Error> {
        self.writes.fetch_add(1, Ordering::Relaxed);
        Err(Error::Io)
    }
    fn load_draft(&self, _: &ControlChain) -> Result<Option<EncryptedObject>, Error> {
        Ok(None)
    }
    fn discard_draft(&self) -> Result<(), Error> {
        // This observer never loaded or owns a sidecar; absence requires no filesystem mutation.
        Ok(())
    }
    fn path(&self) -> &Path {
        Path::new("")
    }
}

fn image_is_local(icon: &IconRef) -> bool {
    matches!(
        icon,
        IconRef::Image {
            source: IconSource::File,
            ..
        }
    )
}

fn require_public_entry(view: &EntryView) {
    // Do not put titles, ordinary fields, protected values or the document into assertion output.
    assert!(
        !view.has_password,
        "unexpected protected field in PUBLIC capture"
    );
    assert!(!view.has_conflicts, "unexpected conflict in PUBLIC capture");
    assert!(view.username.is_none() && view.url.is_none() && view.notes.is_none());
    assert!(view.attributes.is_empty() && view.attachments.is_empty() && view.tags.is_empty());
    assert!(view.expires_at.is_none());
    assert!(view.appearance.foreground.is_none() && view.appearance.background.is_none());
    assert!(
        image_is_local(&view.appearance.icon),
        "PUBLIC image reference is missing"
    );
}

#[test]
fn captured_public_eager_fixture_is_authenticated_and_has_seventy_real_image_entries() {
    // Bind the exact owned descriptor BEFORE any password authentication or decoded document read.
    let mut source = File::open(CAPTURE).expect("PUBLIC captured fixture is unavailable");
    assert!(source.metadata().unwrap().is_file());
    assert_eq!(source.metadata().unwrap().len(), BYTES);
    let mut ciphertext = Vec::with_capacity(BYTES as usize);
    (&mut source)
        .take(BYTES + 1)
        .read_to_end(&mut ciphertext)
        .unwrap();
    assert_eq!(ciphertext.len() as u64, BYTES);
    assert!(
        Digest::of(&ciphertext).to_string() == SHA256,
        "PUBLIC capture checksum mismatch"
    );
    drop(ciphertext);

    let temporary = tempfile::tempdir().unwrap();
    let started = Instant::now();
    let snapshot = ArchiveSnapshot::from_file(
        source,
        None,
        TemporaryStorage::in_directory(temporary.path().to_owned()),
    )
    .unwrap();
    assert!(snapshot.fingerprint().to_string() == SHA256);
    assert_eq!(snapshot.length(), BYTES);
    eprintln!(
        "PUBLIC eager audit phase=archive elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let observed = snapshot.clone();
    let writes = Arc::new(AtomicUsize::new(0));
    let mut service = DatabaseService::new();
    let started = Instant::now();
    let session = service
        .open_managed(
            Box::new(ReadOnlyCapture {
                snapshot,
                writes: Arc::clone(&writes),
            }),
            PASSWORD,
            || Ok(None),
        )
        .unwrap();
    eprintln!(
        "PUBLIC eager audit phase=authenticate elapsed_ms={}",
        started.elapsed().as_millis()
    );

    // This is the same sequence Native overview + Kotlin refresh perform, without a writable author.
    let started = Instant::now();
    let info = service.database_info(&session).unwrap();
    assert!(info.name == "PUBLIC image list visibility" && info.description.is_none());
    assert!(!info.writable && !info.managing && !info.metadata_conflict);
    eprintln!(
        "PUBLIC eager audit phase=database_info elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let started = Instant::now();
    let groups = service.group_info(&session).unwrap();
    assert_eq!(groups.len(), 1);
    let group = &groups[0];
    assert!(group.group.name == "PUBLIC image list group");
    assert!(
        group.group.parent.is_none() && group.description.is_none() && !group.description_conflict
    );
    assert_eq!(group.entry_count, 70);
    assert!(image_is_local(&group.group.icon));
    eprintln!(
        "PUBLIC eager audit phase=group_info rows=70 elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let started = Instant::now();
    let entries = service.entries(&session, None, "").unwrap().value;
    assert_eq!(entries.len(), 70);
    assert_eq!(
        entries
            .iter()
            .map(|entry| &entry.id)
            .collect::<BTreeSet<_>>()
            .len(),
        70
    );
    let expected: BTreeSet<_> = (0..70)
        .map(|index| format!("PUBLIC image {index:03}"))
        .collect();
    assert!(
        entries
            .iter()
            .map(|entry| entry.title.clone())
            .collect::<BTreeSet<_>>()
            == expected
    );
    assert!(entries.iter().all(|entry| entry.group_id == group.group.id && image_is_local(&entry.appearance.icon)));
    assert_eq!(
        entries
            .iter()
            .filter_map(|entry| entry.appearance.icon.blob())
            .collect::<BTreeSet<_>>()
            .len(),
        1
    );
    eprintln!(
        "PUBLIC eager audit phase=entries rows=70 elapsed_ms={}",
        started.elapsed().as_millis()
    );
    let started = Instant::now();
    assert!(service.drafts(&session).unwrap().value.is_empty());
    eprintln!(
        "PUBLIC eager audit phase=forms elapsed_ms={}",
        started.elapsed().as_millis()
    );

    let started = Instant::now();
    let group_preview = service
        .icon_preview(&session, &BinaryTarget::Group(group.group.id.clone()))
        .unwrap()
        .unwrap();
    assert!(
        group_preview.bytes.as_slice() == SVG,
        "PUBLIC group original image differs"
    );
    for row in entries {
        let view = service.view_entry(&session, &row.id).unwrap().value;
        require_public_entry(&view);
        assert!(view.title == row.title);
        let history = service.history(&session, &row.id).unwrap().value;
        assert_eq!(history.len(), 1);
        assert!(history[0].title == row.title);
        let saved = service
            .revision(&session, &row.id, &history[0].id)
            .unwrap()
            .value;
        require_public_entry(&saved);
        assert!(saved.title == row.title);
        let preview = service
            .icon_preview(&session, &BinaryTarget::Entry(row.id))
            .unwrap()
            .unwrap();
        assert!(
            preview.bytes.as_slice() == SVG,
            "PUBLIC entry original image differs"
        );
    }
    eprintln!(
        "PUBLIC eager audit phase=originals_and_history rows=70 elapsed_ms={}",
        started.elapsed().as_millis()
    );
    service.lock_all_checked().unwrap();
    assert_eq!(writes.load(Ordering::Relaxed), 0);
    // Verify the captured inode again; neither reads nor lock may publish a new candidate.
    let mut after = Vec::with_capacity(BYTES as usize);
    observed.copy_ciphertext(&mut after).unwrap();
    assert!(
        Digest::of(&after).to_string() == SHA256,
        "PUBLIC capture was modified"
    );
}
