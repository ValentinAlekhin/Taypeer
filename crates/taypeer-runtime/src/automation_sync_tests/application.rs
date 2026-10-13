//! Real encrypted delivery and storage failures, observed without the worker owner lock.
use super::*;

#[test]
fn observer_updates_without_the_command_owner_and_cannot_reveal_a_revoked_generation() {
    let mut fixture = Fixture::new(1);
    let worker = Mutex::new(fixture.worker(0, &Launcher::default()));
    let owner = worker.lock().unwrap();
    let observer = owner.application_observer();
    let before = observer.snapshot().unwrap();
    assert_eq!(before.phase, ApplicationPhase::Applied);
    assert_eq!(before.pending, 0);
    assert_eq!(before.applied_revision, 0);
    let object = fixture.group("PUBLIC observer delivery");
    fixture.deliver(0, object);
    // Android keeps its Worker in a command mutex. Holding that owner must not
    // block observing automatic application on the independent IPC actor.
    wait(|| {
        observer.snapshot().is_some_and(|state| {
            state.revision > before.revision
                && state.applied_revision > before.applied_revision
                && state.phase == ApplicationPhase::Applied
                && state.pending == 0
        })
    });
    let after = observer.snapshot().unwrap();
    assert_eq!(after.phase, ApplicationPhase::Applied);
    assert_eq!(after.pending, 0);
    assert!(after.applied_revision > before.applied_revision);
    assert!(after.applied_revision <= after.revision);
    owner.control().invalidate(LockReason::Background);
    assert!(observer.snapshot().is_none());
    drop(owner);
    let mut owner = worker.into_inner().unwrap();
    owner.close_report().unwrap();
    assert!(observer.snapshot().is_none());
    drop(owner);

    let mut reopened = fixture.worker(0, &Launcher::default());
    assert_eq!(
        groups(&mut reopened)[0].group.name,
        "PUBLIC observer delivery"
    );
    let fresh = reopened.application_observer().snapshot().unwrap();
    assert_eq!(fresh.applied_revision, 0);
    // Opening another generation cannot reactivate a handle from the old one.
    assert!(observer.snapshot().is_none());
    reopened.close().unwrap();
}

#[test]
fn observer_reports_storage_failure_without_acknowledging_application_and_tracks_recovery() {
    let mut fixture = Fixture::new(1);
    let mut worker = fixture.worker(0, &Launcher::default());
    let observer = worker.application_observer();
    let before = observer.snapshot().unwrap();
    let object = fixture.group("PUBLIC observer storage retry");
    let marker = fixture.receivers[0].path.with_extension("fail-before");
    std::fs::write(&marker, b"PUBLIC application storage fault").unwrap();
    fixture.deliver(0, object);
    wait(|| {
        observer
            .snapshot()
            .is_some_and(|state| state.phase == ApplicationPhase::Failed)
    });
    let failed = observer.snapshot().unwrap();
    assert_eq!(failed.applied, 0);
    assert_eq!(failed.pending, 0);
    assert_eq!(failed.applied_revision, before.applied_revision);
    assert!(groups(&mut worker).is_empty());
    std::fs::remove_file(marker).unwrap();
    wait(|| {
        observer.snapshot().is_some_and(|state| {
            state.revision > failed.revision
                && state.applied_revision > failed.applied_revision
                && state.phase == ApplicationPhase::Applied
                && state.pending == 0
        })
    });
    let rows = groups(&mut worker);
    assert_eq!(rows[0].group.name, "PUBLIC observer storage retry");
    assert_eq!(
        worker
            .request(&Command::GroupHistory(rows[0].group.id.clone()))
            .unwrap()
            .as_array()
            .unwrap()
            .len(),
        1
    );
    worker.close().unwrap();
}

#[test]
fn applied_watermark_survives_missed_empty_pending_and_failed_reports() {
    let fixture = Fixture::new(1);
    let mut worker = fixture.worker(0, &Launcher::default());
    // Use the real generation's access guard but an independently owned
    // publication stream, so automatic worker polling cannot race these reports.
    let observer = WorkerApplicationObserver::new(worker.control());
    assert_eq!(observer.snapshot().unwrap(), ApplicationSnapshot::default());
    let report = |changed, applied, pending| {
        Ok(serde_json::to_value(taypeer_services::ApplyReport {
            changed,
            applied,
            completed: 0,
            pending,
        })
        .unwrap())
    };
    observer.publish(1, &report(false, 0, Vec::new()));
    observer.publish(2, &Err(RuntimeError::Transport));
    assert_eq!(observer.snapshot().unwrap().applied_revision, 0);

    // Legacy reports have original counts but no `changed` field.
    observer.publish(
        3,
        &Ok(serde_json::json!({"applied": 1, "completed": 0, "pending": []})),
    );
    // The consumer does not poll the positive report. Every subsequent outcome
    // still carries its watermark, independent of the latest report's phase.
    observer.publish(4, &report(false, 0, Vec::new()));
    observer.publish(
        5,
        &report(
            false,
            0,
            vec![taypeer_services::PendingPacket {
                object: Digest::of(b"PUBLIC pending application watermark"),
                reason: taypeer_services::PendingReason::Dependency,
            }],
        ),
    );
    let pending = observer.snapshot().unwrap();
    assert_eq!(pending.phase, ApplicationPhase::Pending);
    assert_eq!(pending.applied, 0);
    assert_eq!(pending.applied_revision, 3);
    observer.publish(6, &Err(RuntimeError::Transport));
    observer.publish(7, &Ok(Value::Null));
    let failed = observer.snapshot().unwrap();
    assert_eq!(failed.revision, 7);
    assert_eq!(failed.phase, ApplicationPhase::Failed);
    assert_eq!(failed.applied, 0);
    assert_eq!(failed.applied_revision, 3);
    observer.publish(8, &report(false, 0, Vec::new()));
    let empty = observer.snapshot().unwrap();
    assert_eq!(empty.phase, ApplicationPhase::Applied);
    assert_eq!(empty.applied, 0);
    assert_eq!(empty.applied_revision, 3);
    observer.publish(9, &report(true, 2, Vec::new()));
    assert_eq!(observer.snapshot().unwrap().applied_revision, 9);

    // A changed same-epoch control baseline is durable state even when it
    // accepts no new original changes. Later unsuccessful polls cannot hide it.
    observer.publish(10, &report(true, 0, Vec::new()));
    let baseline = observer.snapshot().unwrap();
    assert_eq!(baseline.applied, 0);
    assert_eq!(baseline.applied_revision, 10);
    observer.publish(11, &report(false, 0, Vec::new()));
    observer.publish(12, &Err(RuntimeError::Transport));
    let after_baseline = observer.snapshot().unwrap();
    assert_eq!(after_baseline.revision, 12);
    assert_eq!(after_baseline.phase, ApplicationPhase::Failed);
    assert_eq!(after_baseline.applied, 0);
    assert_eq!(after_baseline.applied_revision, 10);

    worker.control().invalidate(LockReason::Background);
    assert!(observer.snapshot().is_none());
    worker.close_report().unwrap();
    assert!(observer.snapshot().is_none());
}
