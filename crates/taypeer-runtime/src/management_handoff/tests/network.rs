//! Real isolated workers and authenticated loopback Iroh; every input is synthetic PUBLIC data.
use super::*;
use crate::{ManagementFailure, ManagementPhase};
use taypeer_sync::{ManagementIntent, RelaySetting};

fn source_path(fixture: &Fixture) -> PathBuf {
    fixture
        .manager
        .context
        .copies
        .lock()
        .unwrap()
        .keys()
        .next()
        .unwrap()
        .clone()
}
fn open_source(fixture: &mut Fixture) -> Worker {
    fixture.service.lock(&fixture.session).unwrap();
    fixture
        .manager
        .open_with_launcher(
            &Launcher::default(),
            &source_path(fixture),
            PASSWORD.into(),
            None,
        )
        .unwrap()
}
fn basis(worker: &mut Worker) -> taypeer_services::OpenManagementBasis {
    let mut basis: Option<taypeer_services::OpenManagementBasis> = None;
    wait(|| {
        if !apply(worker) {
            return false;
        }
        basis = serde_json::from_value(worker.request(&Command::ManagementBasis).unwrap()).unwrap();
        basis.is_some()
    });
    basis.unwrap()
}
fn apply(worker: &mut Worker) -> bool {
    match worker.request(&Command::ApplyReceived) {
        Ok(_) => true,
        // A real simultaneous ciphertext receipt can replace the expected generation.
        // Reissue only this same application command; no other failure is retried.
        Err(RuntimeError::Service(taypeer_services::ServiceError::Storage(
            taypeer_storage::Error::Changed,
        ))) => false,
        Err(error) => panic!("PUBLIC application fixture failed: {error:?}"),
    }
}
fn propose(
    worker: &mut Worker,
    host: &RuntimeHost,
    recipient: taypeer_trust::DeviceId,
    operation: Digest,
) -> crate::ManagementHandoff {
    let session = worker.management_session().unwrap();
    let mut result = None;
    wait(|| {
        if !apply(worker) {
            return false;
        }
        match host.propose_management(&session, recipient, operation) {
            Ok(row) => {
                result = Some(row);
                true
            }
            Err(RuntimeError::Management(ManagementFailure::PendingApply)) => false,
            Err(error) => panic!("PUBLIC proposal fixture failed: {error:?}"),
        }
    });
    result.unwrap()
}
fn accept(
    source: &mut Worker,
    recipient: &mut Worker,
    host: &RuntimeHost,
    intent: &ManagementIntent,
) -> crate::ManagementHandoff {
    let session = recipient.management_session().unwrap();
    let mut result = None;
    wait(|| {
        if !apply(source) || !apply(recipient) {
            return false;
        }
        match host.accept_management(&session, intent) {
            Ok(row) => {
                result = Some(row);
                true
            }
            Err(RuntimeError::Management(ManagementFailure::PendingApply)) => false,
            Err(error) => panic!("PUBLIC acceptance fixture failed: {error:?}"),
        }
    });
    result.unwrap()
}
fn complete(source: &RuntimeHost, recipient: &RuntimeHost, intent: &ManagementIntent) {
    wait(|| {
        let row = source.retry_management(intent).unwrap();
        if row.phase != ManagementPhase::Completed {
            recipient.retry_management(intent).unwrap();
        }
        row.phase == ManagementPhase::Completed
    });
}
fn start(fixture: &Fixture) -> EndpointAddr {
    let source = fixture
        .manager
        .start_network(RelaySetting::Disabled)
        .unwrap();
    for receiver in &fixture.receivers {
        receiver.host.start_network(RelaySetting::Disabled).unwrap();
        receiver
            .host
            .exchange(source.clone(), fixture.session.database.clone())
            .unwrap();
    }
    source
}

#[test]
fn explicit_handoff_requires_both_actual_workers_and_authenticated_source_then_applies_once() {
    let mut fixture = Fixture::new(2);
    fixture.group("PUBLIC management retained group");
    start(&fixture);
    let mut source = open_source(&mut fixture);
    let source_session = source.management_session().unwrap();
    let initial = basis(&mut source);
    let chosen = fixture.receivers[0]
        .host
        .profile()
        .identity()
        .unwrap()
        .unwrap()
        .device;
    let original = fixture.snapshot();
    let operation = Digest::of(b"PUBLIC actual two-sided handoff");
    let proposed = propose(&mut source, &fixture.manager, chosen, operation);
    assert_eq!(proposed.phase, ManagementPhase::Proposed);
    assert!(
        fixture.receivers[0]
            .host
            .management_handoffs(&source_session.database)
            .unwrap()
            .is_empty()
    );
    assert!(
        fixture
            .manager
            .management_status(&proposed.intent)
            .unwrap()
            .readiness
            == taypeer_sync::ManagementReadiness::Locked
    );
    let mut recipient = fixture.worker(0, &Launcher::default());
    assert_eq!(basis(&mut recipient).identity.device, chosen);
    assert_eq!(
        propose(&mut source, &fixture.manager, chosen, operation).intent,
        proposed.intent
    );
    assert_eq!(
        fixture.receivers[0]
            .host
            .management_handoffs(&source_session.database)
            .unwrap()[0]
            .phase,
        ManagementPhase::WaitingDecision
    );
    let before = fixture.receivers[0]
        .host
        .management_handoffs(&source_session.database)
        .unwrap();
    let witness = &fixture.receivers[1].host;
    let node = witness.node_for(&source_session.database).unwrap();
    let error = witness
        .runtime
        .block_on(node.request(
            fixture.receivers[0].host.network_address().unwrap(),
            NetworkCommand::ManagementProposal {
                intent: proposed.intent.clone(),
                challenge: Digest::of(b"PUBLIC forged third-device attempt"),
                address: node.address(),
            },
        ))
        .err()
        .expect("third admitted device must not be able to impersonate the manager");
    assert_eq!(error, taypeer_sync::Error::Unauthorized);
    assert_eq!(
        fixture.receivers[0]
            .host
            .management_handoffs(&source_session.database)
            .unwrap(),
        before
    );
    let accepted = accept(
        &mut source,
        &mut recipient,
        &fixture.receivers[0].host,
        &proposed.intent,
    );
    assert_eq!(accepted.phase, ManagementPhase::ConsentRecorded);
    assert!(accepted.applied.is_none());
    complete(
        &fixture.manager,
        &fixture.receivers[0].host,
        &proposed.intent,
    );
    let final_snapshot = fixture.snapshot();
    assert_eq!(
        final_snapshot.chain().head().sequence,
        original.chain().head().sequence + 1
    );
    assert_eq!(
        final_snapshot.chain().head().epoch,
        original.chain().head().epoch
    );
    assert_eq!(final_snapshot.chain().head().manager, chosen);
    assert_eq!(basis(&mut recipient).manager, chosen);
    assert_eq!(basis(&mut source).identity.device, initial.manager);
    assert!(matches!(
        source.request(&Command::CreateInvitation),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Unauthorized
        ))
    ));
    for worker in [&mut source, &mut recipient] {
        let rows = groups(worker);
        assert_eq!(rows.len(), 1);
        assert_eq!(
            worker
                .request(&Command::GroupHistory(rows[0].group.id.clone()))
                .unwrap()
                .as_array()
                .unwrap()
                .len(),
            1
        );
    }
    let bytes = std::fs::read(source_path(&fixture)).unwrap();
    source.close().unwrap();
    assert_eq!(
        fixture
            .manager
            .retry_management(&proposed.intent)
            .unwrap()
            .phase,
        ManagementPhase::Completed
    );
    assert_eq!(std::fs::read(source_path(&fixture)).unwrap(), bytes);
    recipient.close().unwrap();
}

#[test]
fn consent_receipt_does_not_substitute_for_open_readiness_after_lock_and_reopen() {
    let mut fixture = Fixture::new(1);
    start(&fixture);
    let mut source = open_source(&mut fixture);
    let mut recipient = fixture.worker(0, &Launcher::default());
    let old_recipient = recipient.management_session().unwrap();
    let recipient_device = basis(&mut recipient).identity.device;
    let row = propose(
        &mut source,
        &fixture.manager,
        recipient_device,
        Digest::of(b"PUBLIC lock-sensitive handoff"),
    );
    accept(
        &mut source,
        &mut recipient,
        &fixture.receivers[0].host,
        &row.intent,
    );
    let status = fixture.manager.management_status(&row.intent).unwrap();
    assert_eq!(status.phase, Some(ManagementPhase::ConsentRecorded));
    assert!(
        matches!(status.readiness, taypeer_sync::ManagementReadiness::Ready { generation, .. } if generation == old_recipient.generation)
    );
    recipient.close().unwrap();
    let status = fixture.manager.management_status(&row.intent).unwrap();
    assert!(status.readiness == taypeer_sync::ManagementReadiness::Locked);
    assert_eq!(status.phase, Some(ManagementPhase::ConsentRecorded));
    let before = fixture.snapshot();
    assert!(matches!(
        fixture.manager.retry_management(&row.intent),
        Err(RuntimeError::Management(ManagementFailure::NeedsUnlock))
    ));
    assert_eq!(
        fixture.snapshot().chain().head_hash().unwrap(),
        before.chain().head_hash().unwrap()
    );
    let mut reopened = fixture.worker(0, &Launcher::default());
    assert_ne!(
        reopened.management_session().unwrap().generation,
        old_recipient.generation
    );
    assert!(matches!(
        fixture.receivers[0]
            .host
            .accept_management(&old_recipient, &row.intent),
        Err(RuntimeError::Management(ManagementFailure::NeedsUnlock))
    ));
    complete(&fixture.manager, &fixture.receivers[0].host, &row.intent);
    source.close().unwrap();
    reopened.close().unwrap();
}

#[test]
fn lost_source_commit_reply_retains_exact_transition_and_retries_without_restoring_old_role() {
    let mut fixture = Fixture::new(1);
    start(&fixture);
    let mut source = open_source(&mut fixture);
    let mut recipient = fixture.worker(0, &Launcher::default());
    let session = source.management_session().unwrap();
    let recipient_device = basis(&mut recipient).identity.device;
    let row = propose(
        &mut source,
        &fixture.manager,
        recipient_device,
        Digest::of(b"PUBLIC lost source durable reply"),
    );
    accept(
        &mut source,
        &mut recipient,
        &fixture.receivers[0].host,
        &row.intent,
    );
    let path = source_path(&fixture);
    let before = fixture.snapshot();
    std::fs::write(
        path.with_extension("fail-after"),
        b"PUBLIC lost commit reply",
    )
    .unwrap();
    assert!(matches!(
        fixture.manager.retry_management(&row.intent),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::CommitUncertain)
        ))
    ));
    let uncertain = fixture.snapshot();
    assert_eq!(uncertain.chain().head().manager, recipient_device);
    assert_eq!(
        uncertain.chain().head().sequence,
        before.chain().head().sequence + 1
    );
    assert_eq!(
        fixture
            .manager
            .management_handoffs(&session.database)
            .unwrap()[0]
            .phase,
        ManagementPhase::Proposed
    );
    assert!(source.request(&Command::CreateInvitation).is_err());
    std::fs::remove_file(path.with_extension("fail-after")).unwrap();
    complete(&fixture.manager, &fixture.receivers[0].host, &row.intent);
    let after = fixture.snapshot();
    assert_eq!(
        after.chain().head_hash().unwrap(),
        uncertain.chain().head_hash().unwrap()
    );
    assert_eq!(
        after.chain().head().sequence,
        before.chain().head().sequence + 1
    );
    assert_eq!(after.chain().head().epoch, before.chain().head().epoch);
    assert_eq!(basis(&mut source).manager, recipient_device);
    assert!(matches!(
        source.request(&Command::CreateInvitation),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Unauthorized
        ))
    ));
    source.close().unwrap();
    recipient.close().unwrap();
}
