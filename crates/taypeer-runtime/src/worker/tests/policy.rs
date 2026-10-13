//! PUBLIC real private-process projections preserve ciphertext and typed publication failures.
use super::*;
use taypeer_core::{DatabasePolicy, EntryId};
use taypeer_services::{DatabasePolicyDetails, ServiceError, StorageError};
use taypeer_storage::KdfCalibration;

fn request<T: serde::de::DeserializeOwned>(client: &Client, command: &Command) -> T {
    serde_json::from_value(client.request(command, false).unwrap()).unwrap()
}

fn stop(client: &Client) {
    client.control.invalidate(LockReason::Manual);
    client.control.wait_closed().unwrap();
}

#[test]
fn policy_commands_use_actual_password_kdf_and_preview_without_file_history_or_draft_writes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC worker policy.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    edit(&client);
    let entry: EntryId = request(
        &client,
        &Command::SaveDraft {
            operation: taypeer_services::new_operation_id().unwrap(),
        },
    );
    client
        .request(&Command::BeginEdit(entry.clone()), false)
        .unwrap();
    client
        .request(
            &Command::PatchDraft(taypeer_services::EntryPatch {
                notes: taypeer_services::FieldUpdate::Set("PUBLIC retained policy notes".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
    client.request(&Command::PersistDrafts, false).unwrap();
    let before = std::fs::read(&path).unwrap();
    let local_before = std::fs::read(draft_path(&path)).unwrap();
    let snapshot = ArchiveSnapshot::open(&path, None).unwrap();
    let checkpoint = snapshot
        .object(snapshot.metadata().manifest.body.checkpoint)
        .unwrap();
    let parameters =
        taypeer_storage::password_kdf_parameters(&checkpoint.password_header().unwrap()).unwrap();
    let details: DatabasePolicyDetails = request(&client, &Command::PolicyDetails);
    assert_eq!(details.protection.parameters, parameters);
    assert!(
        details
            .protection
            .password_derivation_ms
            .is_some_and(|elapsed| elapsed > 0)
    );
    assert_eq!(details.usage.file_bytes, before.len() as u64);
    let preview: KdfCalibration =
        request(&client, &Command::CalibrateProtection { target_ms: 500 });
    assert_eq!(preview.target_ms, 500);
    assert_eq!(preview.measurement.parameters.memory_kib, 65_536);
    assert_eq!(preview.measurement.parameters.parallelism, 1);
    assert!(preview.measurement.elapsed_ms > 0);
    assert!((3..=256).contains(&preview.measurement.parameters.iterations));
    assert!(matches!(
        client.request(&Command::CalibrateProtection { target_ms: 499 }, false),
        Err(RuntimeError::Service(ServiceError::InvalidInput))
    ));
    let after: DatabasePolicyDetails = request(&client, &Command::PolicyDetails);
    assert_eq!(
        after.protection.password_derivation_ms,
        details.protection.password_derivation_ms
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(draft_path(&path)).unwrap(), local_before);
    let history: Vec<taypeer_services::RevisionSummary> =
        request(&client, &Command::History(entry));
    assert_eq!(history.len(), 1);
    stop(&client);
    let readonly = open_mode(&sessions, &path, false, true);
    let _: DatabasePolicyDetails = request(&readonly, &Command::PolicyDetails);
    assert!(matches!(
        readonly.request(&Command::CalibrateProtection { target_ms: 500 }, false),
        Err(RuntimeError::Service(ServiceError::ReadOnly))
    ));
    assert_eq!(std::fs::read(&path).unwrap(), before);
    stop(&readonly);
}

#[test]
fn policy_io_and_postpublication_unknown_remain_typed_and_exact_retry_after_restart_is_once() {
    for (suffix, expected) in [
        ("fail-before", StorageError::Io),
        ("fail-after", StorageError::CommitUncertain),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC policy failure.taypeer");
        let sessions = SessionController::new(SessionPolicy::default());
        let client = open(&sessions, &path, true);
        let applied: DatabasePolicy = request(&client, &Command::DatabasePolicy);
        let policy =
            DatabasePolicy::new(512 * 1024, 2 * 1024 * 1024, applied.kdf_target_ms()).unwrap();
        let operation = Digest::of(b"PUBLIC exact policy publication");
        let command = Command::SetDatabasePolicy {
            policy,
            operation,
            password: None,
        };
        let before = std::fs::read(&path).unwrap();
        std::fs::write(
            path.with_extension(suffix),
            b"PUBLIC typed publication fault",
        )
        .unwrap();
        assert!(matches!(
            client.request(&command, false),
            Err(RuntimeError::Service(ServiceError::Storage(actual))) if actual == expected
        ));
        let uncertain = std::fs::read(&path).unwrap();
        assert_eq!(uncertain == before, suffix == "fail-before");
        std::fs::remove_file(path.with_extension(suffix)).unwrap();
        stop(&client);
        let reopened = open(&sessions, &path, false);
        reopened.request(&command, false).unwrap();
        let durable = std::fs::read(&path).unwrap();
        if suffix == "fail-after" {
            assert_eq!(durable, uncertain);
        }
        reopened.request(&command, false).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), durable);
        assert_eq!(
            request::<DatabasePolicy>(&reopened, &Command::DatabasePolicy),
            policy
        );
        stop(&reopened);
    }
}
