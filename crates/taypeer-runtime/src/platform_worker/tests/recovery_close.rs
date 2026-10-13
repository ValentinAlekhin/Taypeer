//! Recovery uses the real supervised descriptor worker and encrypted source/candidate files.
use super::*;
use crate::{RecoveryIntent, RecoveryPublication};
use taypeer_services::{DatabaseService, NamedRecoveryIntent};
use taypeer_trust::Identity;

fn publication(fixture: &Fixture) -> RecoveryPublication {
    let mut service = DatabaseService::new();
    let session = fixture
        .host
        .open_local(&mut service, &fixture.path, PASSWORD.as_bytes(), None)
        .unwrap();
    let captured = service.freeze_trust_recovery_source(&session).unwrap();
    let original = fixture.launcher.writer.snapshot().unwrap();
    let operation = Digest::of(b"PUBLIC actual descriptor recovery closure");
    let (source, _) = fixture
        .launcher
        .writer
        .freeze_recovery(operation, original.fingerprint())
        .unwrap();
    let author = fixture.launcher.writer.author_recovery(&source).unwrap();
    let identity =
        Identity::new(author.public(), fixture.launcher.writer.transport_public()).unwrap();
    let intent = RecoveryIntent { source, identity };
    let named = NamedRecoveryIntent {
        operation,
        database: intent.source.database.clone(),
        working_copy: intent.source.working_copy,
        root: intent.source.root,
        source: intent.source.fingerprint,
        identity,
        name: "PUBLIC descriptor recovered copy".into(),
    };
    let seed = captured
        .bind_named(named, b"PUBLIC descriptor recovery password")
        .unwrap()
        .prepare(&author)
        .unwrap();
    let value = fixture
        .launcher
        .writer
        .publish_recovery(&intent, seed)
        .unwrap();
    fixture.launcher.writer.confirm_recovery(&value).unwrap();
    service.lock(&session).unwrap();
    value
}

fn busy(result: Result<crate::WorkingCopy, RuntimeError>) {
    assert!(matches!(
        result,
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(StorageError::Busy)
        ))
    ));
}

#[test]
fn real_platform_exit_and_released_capabilities_unregister_only_the_exact_recovery_source() {
    let fixture = Fixture::new();
    let mut worker = fixture.open(PASSWORD, true).unwrap();
    fixture.host.register_working_copy(&fixture.path).unwrap();
    edit(&mut worker, "PUBLIC recovery closure entry");
    let captured = editor(&mut worker);
    save(
        &mut worker,
        &captured,
        taypeer_services::new_operation_id().unwrap(),
    )
    .unwrap();
    let value = publication(&fixture);
    let source = fixture.host.working_copies().unwrap();
    let before = std::fs::read(&fixture.path).unwrap();
    busy(fixture.host.adopt_closed_platform_recovery(&value));
    assert!(worker.request(&Command::DatabaseInfo).is_ok());
    assert_eq!(fixture.host.working_copies().unwrap(), source);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);

    worker.close().unwrap();
    assert_eq!(
        worker.session_status().phase,
        crate::session::SessionPhase::Closed
    );
    assert_eq!(
        fixture
            .launcher
            .faults
            .confirmed_exits
            .load(Ordering::Acquire),
        1
    );
    // Exit is necessary but not sufficient: this capability can still open the source.
    busy(fixture.host.adopt_closed_platform_recovery(&value));
    assert!(fixture.launcher.writer.snapshot().is_ok());
    let Fixture {
        host,
        launcher,
        path,
        _directory,
        credentials,
    } = fixture;
    let faults = Arc::clone(&launcher.faults);
    drop(launcher);
    // The actual callback server releases its writer after the worker stream closes.
    wait(|| faults.released_callback_servers.load(Ordering::Acquire) == 1);
    busy(host.adopt_platform_recovery(&value));

    let mut wrong = value.clone();
    wrong.fingerprint = Digest::of(b"PUBLIC forged candidate before coordinator removal");
    assert!(host.adopt_closed_platform_recovery(&wrong).is_err());
    assert!(
        host.coordinator()
            .snapshot(&value.intent.source.database)
            .is_ok()
    );
    assert_eq!(host.working_copies().unwrap(), source);

    let reopened = host.platform_cipher_writer(&path).unwrap();
    assert!(reopened.snapshot().is_ok());
    busy(host.adopt_closed_platform_recovery(&value));
    drop(reopened);
    let mut local = DatabaseService::new();
    let session = host
        .open_local(&mut local, &path, PASSWORD.as_bytes(), None)
        .unwrap();
    busy(host.adopt_closed_platform_recovery(&value));
    local
        .create_group(
            &session,
            "PUBLIC source writer still works after rejected adoption".into(),
            None,
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    local.lock(&session).unwrap();
    drop(local);
    let unchanged = std::fs::read(&path).unwrap();
    let copy = host.adopt_closed_platform_recovery(&value).unwrap();
    assert_eq!(copy.database, source[0].database);
    assert_ne!(copy.root, source[0].root);
    assert_eq!(host.working_copies().unwrap(), vec![copy.clone()]);
    assert_eq!(host.adopt_closed_platform_recovery(&value).unwrap(), copy);
    assert_eq!(std::fs::read(&path).unwrap(), unchanged);
    assert_eq!(faults.forced_terminations.load(Ordering::Acquire), 0);
    drop(worker);
    drop(host);
    let cold = RuntimeHost::with_platform_credentials(
        &_directory.path().join("profile"),
        SessionController::new(Default::default()),
        credentials,
    )
    .unwrap();
    assert_eq!(cold.adopt_closed_platform_recovery(&value).unwrap(), copy);
    assert_eq!(std::fs::read(&path).unwrap(), unchanged);
}
