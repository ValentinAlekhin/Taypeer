//! Synthetic stuck storage call inside a real plaintext child.
use super::*;
use std::time::{Duration, Instant};
use taypeer_storage::{CipherPersistence, EncryptedObject, Error, PreparedCommit};
use taypeer_trust::ControlChain;

pub(super) struct Persistence(pub CoordinatorPersistence);
impl CipherPersistence for Persistence {
    fn snapshot(&self) -> Result<ArchiveSnapshot, Error> {
        if self.path().with_extension("block").exists() {
            std::fs::write(self.path().with_extension("busy"), b"PUBLIC blocked").unwrap();
            std::thread::sleep(Duration::from_secs(60));
        }
        self.0.snapshot()
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, Error> {
        self.0.commit(request)
    }
    fn working_copy(&self) -> Digest {
        self.0.working_copy()
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), Error> {
        self.0.save_draft(object)
    }
    fn load_draft(&self, chain: &ControlChain) -> Result<Option<EncryptedObject>, Error> {
        self.0.load_draft(chain)
    }
    fn discard_draft(&self) -> Result<(), Error> {
        self.0.discard_draft()
    }
    fn path(&self) -> &Path {
        self.0.path()
    }
}

#[test]
fn forced_shutdown_preserves_the_last_durable_draft_and_archive() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let first = open(&sessions, &path, true);
    edit(&first);
    first.control.invalidate(LockReason::Manual);
    assert_eq!(
        first.control.wait_closed().unwrap().draft,
        DraftDisposition::Preserved
    );
    let archive = std::fs::read(&path).unwrap();
    let draft = std::fs::read(draft_path(&path)).unwrap();
    let next = open(&sessions, &path, false);
    next.request(&Command::RestoreDraft, false).unwrap();
    next.request(
        &Command::PatchDraft(taypeer_services::EntryPatch {
            title: taypeer_services::FieldUpdate::Set("PUBLIC latest unsaved title".into()),
            ..Default::default()
        }),
        false,
    )
    .unwrap();
    std::fs::write(path.with_extension("block"), b"PUBLIC block").unwrap();
    let running = Arc::clone(&next);
    let request = std::thread::spawn(move || running.request(&Command::Groups, false));
    let until = Instant::now() + Duration::from_secs(5);
    while !path.with_extension("busy").exists() {
        assert!(Instant::now() < until);
        std::thread::sleep(Duration::from_millis(10));
    }
    sessions.lock_all(LockReason::Sleep);
    assert_eq!(
        request.join().unwrap(),
        Err(RuntimeError::OperationInterrupted(LockReason::Sleep))
    );
    let outcome = next.control.wait_closed().unwrap();
    assert_eq!(outcome.termination, Termination::Forced);
    assert_eq!(outcome.draft, DraftDisposition::Unconfirmed);
    assert_eq!(std::fs::read(&path).unwrap(), archive);
    assert_eq!(std::fs::read(draft_path(&path)).unwrap(), draft);
}
