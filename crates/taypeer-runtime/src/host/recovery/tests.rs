//! PUBLIC-only fault fixtures exercise real archives and protected recovery receipts.
use super::*;
use crate::profile::{CredentialStore, ProfileError};
use taypeer_services::{DatabaseService, NamedRecoveryIntent, SessionToken};
use zeroize::Zeroizing;

mod boot;

type CredentialValues = BTreeMap<(String, String), Vec<u8>>;
#[derive(Clone, Copy)]
enum Fault {
    Before,
    After,
}
struct Rule {
    account: String,
    phase: Option<&'static str>,
    fault: Fault,
}
#[derive(Default)]
struct Credentials {
    values: Mutex<CredentialValues>,
    fault: Mutex<Option<Rule>>,
}
impl Credentials {
    fn fail(&self, account: &str, phase: Option<&'static str>, fault: Fault) {
        *self.fault.lock().unwrap() = Some(Rule {
            account: account.into(),
            phase,
            fault,
        });
    }
    fn replace(&self, account: &str, bytes: Vec<u8>) {
        let mut values = self.values.lock().unwrap();
        let key = values
            .keys()
            .find(|(_, key)| key == &format!("state:{account}"))
            .unwrap()
            .clone();
        values.insert(key, bytes);
    }
}
impl CredentialStore for Credentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned()
            .map(Zeroizing::new))
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        let fault = {
            let mut rule = self.fault.lock().unwrap();
            if rule.as_ref().is_some_and(|rule| {
                account
                    .strip_prefix("state:")
                    .unwrap_or(account)
                    .starts_with(&rule.account)
                    && rule.phase.is_none_or(|phase| {
                        serde_json::from_slice::<serde_json::Value>(bytes)
                            .ok()
                            .and_then(|value| {
                                value
                                    .get("phase")
                                    .and_then(|value| value.as_str())
                                    .map(str::to_owned)
                            })
                            .as_deref()
                            == Some(phase)
                    })
            }) {
                rule.take().map(|rule| rule.fault)
            } else {
                None
            }
        };
        if matches!(fault, Some(Fault::Before)) {
            return Err(ProfileError::Io);
        }
        self.values
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), bytes.to_vec());
        if matches!(fault, Some(Fault::After)) {
            return Err(ProfileError::CommitUncertain);
        }
        Ok(())
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    credentials: Arc<Credentials>,
    host: RuntimeHost,
    service: DatabaseService,
    session: SessionToken,
    writer: Arc<PlatformCipherWriter>,
    path: PathBuf,
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
        let path = directory.path().join("PUBLIC source.taypeer");
        let mut service = DatabaseService::new();
        let session = host
            .open_local(
                &mut service,
                &path,
                b"PUBLIC original password",
                Some("PUBLIC original name".into()),
            )
            .unwrap();
        host.register_working_copy(&path).unwrap();
        let writer = host.platform_cipher_writer(&path).unwrap();
        Self {
            directory,
            credentials,
            host,
            service,
            session,
            writer,
            path,
        }
    }
    fn prepare(&self, operation: Digest) -> Prepared {
        let captured = self
            .service
            .freeze_trust_recovery_source(&self.session)
            .unwrap();
        let original = self.writer.snapshot().unwrap();
        let (source, archive) = self
            .writer
            .freeze_recovery(operation, original.fingerprint())
            .unwrap();
        assert_eq!(archive.fingerprint(), captured.snapshot().fingerprint());
        let author = self.writer.author_recovery(&source).unwrap();
        let identity = Identity::new(author.public(), self.writer.transport_public()).unwrap();
        let intent = RecoveryIntent { source, identity };
        let named = NamedRecoveryIntent {
            operation,
            database: intent.source.database.clone(),
            working_copy: intent.source.working_copy,
            root: intent.source.root,
            source: intent.source.fingerprint,
            identity,
            name: "PUBLIC frozen recovered name".into(),
        };
        let seed = captured
            .bind_named(named.clone(), b"PUBLIC new recovery password")
            .unwrap()
            .prepare(&author)
            .unwrap();
        Prepared {
            intent,
            named,
            original: archive,
            seed,
        }
    }
}
struct Prepared {
    intent: RecoveryIntent,
    named: NamedRecoveryIntent,
    original: ArchiveSnapshot,
    seed: ArchiveSeed,
}
fn uncertain(error: RuntimeError) -> bool {
    error == cipher_ipc::storage(taypeer_storage::Error::CommitUncertain)
}

#[test]
fn proof_and_actual_source_writer_closure_are_required_before_idempotent_adoption() {
    let mut fixture = Fixture::new();
    let before = std::fs::read(&fixture.path).unwrap();
    let prepared = fixture.prepare(Digest::of(b"PUBLIC recovery adoption"));
    let value = fixture
        .writer
        .publish_recovery(&prepared.intent, prepared.seed)
        .unwrap();
    assert_eq!(
        fixture.host.recovery_phase(&value.intent.source).unwrap(),
        Some(RecoveryPhase::CandidatePublished)
    );
    assert!(fixture.host.adopt_platform_recovery(&value).is_err());
    let candidate = fixture
        .writer
        .recovery_candidate(&value.intent)
        .unwrap()
        .unwrap();
    assert_eq!(
        taypeer_services::validate_named_trust_recovery(
            &prepared.original,
            &candidate,
            &prepared.named,
            b"PUBLIC new recovery password"
        )
        .unwrap(),
        value.root
    );
    fixture.writer.confirm_recovery(&value).unwrap();
    fixture.writer.confirm_recovery(&value).unwrap();
    assert_eq!(
        fixture.host.pending_recoveries().unwrap()[0].phase,
        RecoveryPhase::ProofConfirmed
    );
    assert_eq!(
        fixture.host.adopt_platform_recovery(&value),
        Err(cipher_ipc::storage(taypeer_storage::Error::Busy))
    );
    fixture.service.lock(&fixture.session).unwrap();
    drop(fixture.service);
    fixture.host.close(&fixture.session.database).unwrap();
    let copy = fixture.host.adopt_platform_recovery(&value).unwrap();
    assert_eq!(fixture.host.adopt_platform_recovery(&value).unwrap(), copy);
    assert!(fixture.host.pending_recoveries().unwrap().is_empty());
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    let profile = fixture.directory.path().join("profile");
    drop(fixture.writer);
    drop(fixture.host);
    let restarted = RuntimeHost::with_platform_credentials(
        &profile,
        crate::session::SessionController::new(Default::default()),
        fixture.credentials,
    )
    .unwrap();
    assert_eq!(restarted.working_copies().unwrap(), vec![copy.clone()]);
    assert_eq!(restarted.adopt_platform_recovery(&value).unwrap(), copy);
}

#[test]
fn a_new_worker_cannot_reuse_a_vanished_prepublication_password_attempt() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(Digest::of(b"PUBLIC vanished password attempt"));
    let replacement = fixture.host.platform_cipher_writer(&fixture.path).unwrap();
    assert!(matches!(
        replacement.recovery_snapshot(prepared.intent.source.operation),
        Err(RuntimeError::RecoveryRequiresNewOperation)
    ));
    assert_eq!(
        fixture
            .host
            .recovery_phase(&prepared.intent.source)
            .unwrap(),
        Some(RecoveryPhase::RefusedBeforePublication)
    );
    assert_eq!(
        fixture.host.pending_recoveries().unwrap()[0].phase,
        RecoveryPhase::RefusedBeforePublication
    );
    assert!(matches!(
        fixture
            .writer
            .publish_recovery(&prepared.intent, prepared.seed),
        Err(RuntimeError::RecoveryRequiresNewOperation)
    ));
    let snapshot = replacement.snapshot().unwrap();
    let new_operation = Digest::of(b"PUBLIC newly explicit recovery attempt");
    let (source, _) = replacement
        .freeze_recovery(new_operation, snapshot.fingerprint())
        .unwrap();
    assert_eq!(source.operation, new_operation);
    assert!(matches!(
        replacement.recovery_snapshot(prepared.intent.source.operation),
        Err(RuntimeError::RecoveryRequiresNewOperation)
    ));
}

#[test]
fn published_candidate_survives_lost_reply_and_retry_uses_original_after_source_edit() {
    let mut fixture = Fixture::new();
    let prepared = fixture.prepare(Digest::of(b"PUBLIC lost encrypted publication reply"));
    fixture.credentials.fail(
        "recovery-attempt:",
        Some("CandidatePublished"),
        Fault::After,
    );
    let error = fixture
        .writer
        .publish_recovery(&prepared.intent, prepared.seed)
        .unwrap_err();
    assert!(uncertain(error));
    let candidate = fixture
        .writer
        .recovery_candidate(&prepared.intent)
        .unwrap()
        .unwrap();
    let value = publication(&prepared.intent, &candidate).unwrap();
    let destination = destination(&fixture.host.context, &prepared.intent.source).unwrap();
    let candidate_bytes = std::fs::read(&destination).unwrap();
    fixture
        .service
        .create_group(
            &fixture.session,
            "PUBLIC changed after capture".into(),
            None,
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    let current = fixture.writer.snapshot().unwrap();
    assert_ne!(current.fingerprint(), prepared.original.fingerprint());
    let replacement = fixture.host.platform_cipher_writer(&fixture.path).unwrap();
    let (source, original) = replacement
        .recovery_snapshot(value.intent.source.operation)
        .unwrap()
        .unwrap();
    assert_eq!(source, value.intent.source);
    assert_eq!(original.fingerprint(), prepared.original.fingerprint());
    let (source, _) = replacement
        .freeze_recovery(source.operation, original.fingerprint())
        .unwrap();
    replacement.author_recovery(&source).unwrap();
    replacement.confirm_recovery(&value).unwrap();
    assert_eq!(std::fs::read(&destination).unwrap(), candidate_bytes);
    let confirmed = fixture.credentials.values.lock().unwrap();
    for ((_, account), bytes) in confirmed
        .iter()
        .filter(|((_, account), _)| account.starts_with("recovery"))
    {
        assert!(
            !bytes
                .windows(prepared.named.name.len())
                .any(|part| part == prepared.named.name.as_bytes()),
            "PUBLIC protected descriptor account {account}"
        );
        assert!(
            !bytes
                .windows(b"PUBLIC new recovery password".len())
                .any(|part| part == b"PUBLIC new recovery password")
        );
    }
}

#[test]
fn proof_prepublication_failure_and_postpublication_unknown_never_fake_an_ack() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(Digest::of(b"PUBLIC proof fault"));
    let value = fixture
        .writer
        .publish_recovery(&prepared.intent, prepared.seed)
        .unwrap();
    fixture.credentials.fail("recovery:", None, Fault::Before);
    assert!(matches!(
        fixture.writer.confirm_recovery(&value),
        Err(RuntimeError::Profile(ProfileError::Io))
    ));
    assert_eq!(
        fixture.host.recovery_phase(&value.intent.source).unwrap(),
        Some(RecoveryPhase::CandidatePublished)
    );
    fixture.credentials.fail("recovery:", None, Fault::After);
    assert!(uncertain(
        fixture.writer.confirm_recovery(&value).unwrap_err()
    ));
    assert_eq!(
        fixture.host.recovery_phase(&value.intent.source).unwrap(),
        Some(RecoveryPhase::CandidatePublished)
    );
    fixture.writer.confirm_recovery(&value).unwrap();
    assert_eq!(
        fixture.host.recovery_phase(&value.intent.source).unwrap(),
        Some(RecoveryPhase::ProofConfirmed)
    );
}

#[test]
fn altered_identity_publication_and_other_source_gateway_refuse_without_mutation() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(Digest::of(b"PUBLIC exact identity recovery"));
    let mut changed = prepared.intent.clone();
    changed.identity = Identity::new(
        changed.identity.author,
        taypeer_trust::TransportKey::from_seed(&[97; 32]).public(),
    )
    .unwrap();
    let attempt_bytes = fixture.credentials.values.lock().unwrap().clone();
    assert!(matches!(
        fixture.writer.recovery_candidate(&changed),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Trust(taypeer_trust::Error::OperationMismatch)
        ))
    ));
    assert_eq!(*fixture.credentials.values.lock().unwrap(), attempt_bytes);
    let value = fixture
        .writer
        .publish_recovery(&prepared.intent, prepared.seed)
        .unwrap();
    let mut wrong = value.clone();
    wrong.fingerprint = Digest::of(b"PUBLIC forged candidate fingerprint");
    assert!(fixture.writer.confirm_recovery(&wrong).is_err());
    let source = fixture
        .writer
        .recovery_candidate(&prepared.intent)
        .unwrap()
        .unwrap();
    assert_eq!(source.fingerprint(), value.fingerprint);
    let other = fixture
        .directory
        .path()
        .join("PUBLIC another source.taypeer");
    std::fs::copy(&fixture.path, &other).unwrap();
    let other_writer = fixture.host.platform_cipher_writer(&other).unwrap();
    assert!(other_writer.author_recovery(&value.intent.source).is_err());
}

#[test]
fn missing_corrupt_and_future_protected_records_are_distinct() {
    let fixture = Fixture::new();
    assert!(fixture.host.pending_recoveries().unwrap().is_empty());
    let prepared = fixture.prepare(Digest::of(b"PUBLIC protected record schema"));
    let account = account(prepared.intent.source.working_copy);
    let attempt = load(&fixture.host.context, prepared.intent.source.working_copy)
        .unwrap()
        .unwrap();
    fixture
        .credentials
        .replace(&account, b"PUBLIC malformed protected record".to_vec());
    assert!(matches!(
        fixture.host.pending_recoveries(),
        Err(RuntimeError::Profile(ProfileError::Invalid))
    ));
    let mut future = attempt;
    future.version = VERSION + 1;
    fixture
        .credentials
        .replace(&account, serde_json::to_vec(&future).unwrap());
    assert!(matches!(
        fixture.host.recovery_phase(&prepared.intent.source),
        Err(RuntimeError::Profile(ProfileError::Invalid))
    ));
}

#[test]
fn a_failed_source_descriptor_write_retries_the_issued_generation_without_using_current_path() {
    let mut fixture = Fixture::new();
    let operation = Digest::of(b"PUBLIC failed first source freeze");
    let original = fixture.writer.snapshot().unwrap();
    fixture
        .credentials
        .fail("recovery-attempt:", None, Fault::Before);
    assert!(matches!(
        fixture
            .writer
            .freeze_recovery(operation, original.fingerprint()),
        Err(RuntimeError::Profile(ProfileError::Io))
    ));
    fixture
        .service
        .create_group(
            &fixture.session,
            "PUBLIC source changed during staging retry".into(),
            None,
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    assert_ne!(
        fixture.writer.snapshot().unwrap().fingerprint(),
        original.fingerprint()
    );
    let (source, frozen) = fixture
        .writer
        .freeze_recovery(operation, original.fingerprint())
        .unwrap();
    assert_eq!(source.fingerprint, original.fingerprint());
    assert_eq!(frozen.fingerprint(), original.fingerprint());
    assert_eq!(
        fixture.host.recovery_phase(&source).unwrap(),
        Some(RecoveryPhase::SourceFrozen)
    );
}
