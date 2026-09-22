//! Real subprocesses reopen frozen PUBLIC inputs; no credentials or current writer setup.
use super::*;

#[test]
fn frozen_read_only_database_and_draft_survive_session_revocation() {
    let corpus = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dev5");
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let archive = std::fs::read(corpus.join("populated.taypeer")).unwrap();
    let draft = std::fs::read(corpus.join("populated.draft")).unwrap();
    std::fs::write(&path, &archive).unwrap();
    std::fs::write(draft_path(&path), &draft).unwrap();
    let expected: serde_json::Value =
        serde_json::from_slice(&std::fs::read(corpus.join("populated.json")).unwrap()).unwrap();
    let entry: taypeer_core::EntryId =
        serde_json::from_value(expected["entries"][0]["id"].clone()).unwrap();
    let sessions = SessionController::new(SessionPolicy::default());
    let reader = open_access(&sessions, &path, false, TestAccess::UnsupportedWriter);
    let report = reader.request(&Command::Compatibility, false).unwrap();
    assert_eq!(report["value"]["read"]["status"], "supported");
    assert_eq!(report["value"]["write"]["status"], "missing_features");
    assert_eq!(
        reader
            .request(&Command::RevealPassword(entry.clone()), false)
            .unwrap(),
        "PUBLIC saved password"
    );
    for command in [Command::BeginEdit(entry.clone()), Command::RestoreDraft] {
        assert_eq!(
            reader.request(&command, false),
            Err(RuntimeError::Service(
                taypeer_services::ServiceError::WriteCompatibility
            ))
        );
    }
    sessions.lock_all(LockReason::SystemLocked);
    assert_eq!(
        reader.control.wait_closed().unwrap().draft,
        DraftDisposition::Preserved
    );
    assert_eq!(std::fs::read(&path).unwrap(), archive);
    assert_eq!(std::fs::read(draft_path(&path)).unwrap(), draft);
    let writer = open(&sessions, &path, false);
    assert_ne!(
        reader.control.status().generation,
        writer.control.status().generation
    );
    assert!(
        reader
            .request(&Command::RevealPassword(entry.clone()), false)
            .is_err()
    );
    assert_eq!(
        writer
            .request(&Command::Entry(entry.clone()), false)
            .unwrap()["title"],
        "PUBLIC entry"
    );
    let status = writer.request(&Command::DraftStatus, false).unwrap();
    assert_eq!(status["active"], false);
    assert_eq!(
        status["pending"]["entry_id"],
        serde_json::to_value(&entry).unwrap()
    );
    writer.request(&Command::RestoreDraft, false).unwrap();
    let saved = writer
        .request(
            &Command::SaveDraft {
                operation: taypeer_services::new_operation_id().unwrap(),
            },
            false,
        )
        .unwrap();
    assert_eq!(saved, serde_json::to_value(&entry).unwrap());
    assert_eq!(
        writer.request(&Command::Entry(entry), false).unwrap()["title"],
        "PUBLIC deferred corpus draft"
    );
    writer.control.invalidate(LockReason::Manual);
    assert_eq!(
        writer.control.wait_closed().unwrap().draft,
        DraftDisposition::Preserved
    );
    let reopened = open(&sessions, &path, false);
    assert!(reopened.request(&Command::DraftStatus, false).unwrap()["pending"].is_null());
    let entry: taypeer_core::EntryId = serde_json::from_value(saved).unwrap();
    assert_eq!(
        reopened.request(&Command::Entry(entry), false).unwrap()["title"],
        "PUBLIC deferred corpus draft"
    );
    reopened.control.invalidate(LockReason::Manual);
    reopened.control.wait_closed().unwrap();
}
