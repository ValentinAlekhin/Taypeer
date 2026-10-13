//! PUBLIC-only registry fixtures; OS/worker proof remains a separate integration.
use super::*;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use std::{
    fs,
    sync::{
        Arc, Mutex,
        atomic::{AtomicU8, AtomicUsize, Ordering},
    },
};
use taypeer_core::DatabaseId;
use tempfile::TempDir;

fn digest(value: u8) -> Digest {
    Digest::from_bytes([value; 32])
}
fn generation(value: u128) -> Generation {
    Generation::parse(&format!("{value:032x}")).unwrap()
}
fn binding(profile: &Path, database: &str, epoch: u64) -> Binding {
    Binding::for_profile(
        profile,
        digest(1),
        DatabaseId::new(database),
        digest(2),
        epoch,
    )
    .unwrap()
}
fn wrapper(binding: &Binding, value: u8) -> WrapperRecord {
    WrapperRecord::new(
        binding.clone(),
        generation(u128::from(value)),
        digest(value),
        [value; 256],
    )
    .unwrap()
}
fn public_proof(binding: &Binding, value: u8, operation: u8) -> ConfirmedEnrollment {
    // This child-only PUBLIC fixture does not claim an OS authentication result.
    ConfirmedEnrollment::after_worker_proof(digest(operation), wrapper(binding, value))
}
struct Fixture {
    profile: TempDir,
    binding: Binding,
    registry: Registry,
}
impl Fixture {
    fn new() -> Self {
        let profile = tempfile::tempdir().unwrap();
        let binding = binding(profile.path(), "PUBLIC database", 1);
        let registry = Registry::new(profile.path(), binding.clone()).unwrap();
        Self {
            profile,
            binding,
            registry,
        }
    }
    fn enable(&mut self, generation: u8, operation: u8) -> WrapperRecord {
        let expected = wrapper(&self.binding, generation);
        let active = load(&self.registry.path, &self.binding)
            .unwrap()
            .and_then(|loaded| loaded.record.active);
        assert_eq!(
            self.registry
                .begin_enable(public_proof(&self.binding, generation, operation)),
            Ok(SetupState::PendingEnable {
                operation: digest(operation),
                candidate: expected.clone(),
                active
            })
        );
        assert_eq!(
            self.registry.finish_enable(digest(operation)),
            Ok(SetupState::Enabled(expected.clone()))
        );
        expected
    }
    fn bytes(&self) -> Vec<u8> {
        fs::read(&self.registry.path).unwrap()
    }
}

// Modes are consumed once. Both error paths execute actual publish_file; the
// pre-rename path targets an owned directory that cannot be replaced by a file.
const BEFORE_RENAME: u8 = 1;
const LOST_ACK: u8 = 2;
#[derive(Clone)]
struct PublicationProbe {
    calls: Arc<AtomicUsize>,
    fault: Arc<AtomicU8>,
    exact: Arc<Mutex<Vec<Vec<u8>>>>,
    blocked: PathBuf,
}
impl PublicationProbe {
    fn install(fixture: &mut Fixture) -> Self {
        let blocked = fixture.profile.path().join("PUBLIC blocked publication");
        fs::create_dir(&blocked).unwrap();
        fs::write(blocked.join("PUBLIC retained"), b"PUBLIC original").unwrap();
        let probe = Self {
            calls: Arc::new(AtomicUsize::new(0)),
            fault: Arc::new(AtomicU8::new(0)),
            exact: Arc::new(Mutex::new(Vec::new())),
            blocked,
        };
        fixture.registry.publisher = Box::new(probe.clone());
        probe
    }
    fn lose_next(&self) {
        self.fault.store(LOST_ACK, Ordering::SeqCst);
    }
    fn fail_next_before_rename(&self) {
        self.fault.store(BEFORE_RENAME, Ordering::SeqCst);
    }
    fn count(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
}
impl Publisher for PublicationProbe {
    fn publish(&self, destination: &Path, bytes: &[u8]) -> Result<(), RegistryError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.exact.lock().unwrap().push(bytes.to_vec());
        match self.fault.swap(0, Ordering::SeqCst) {
            BEFORE_RENAME => FilePublisher.publish(&self.blocked, bytes),
            LOST_ACK => {
                FilePublisher.publish(destination, bytes)?;
                // A real durable publication followed by a lost final reply.
                Err(RegistryError::CommitUncertain)
            }
            _ => FilePublisher.publish(destination, bytes),
        }
    }
}

#[test]
fn worst_escaped_database_active_enable_receipt_fits_canonical_limit() {
    let profile = tempfile::tempdir().unwrap();
    let binding = binding(profile.path(), &"\0".repeat(128), u64::MAX);
    let active = wrapper(&binding, 3);
    let candidate = wrapper(&binding, 4);
    let mut record = Record::empty();
    record.active = Some(active.clone());
    record.pending = Some(PendingSetup::enable(digest(9), candidate.clone()));
    record.receipt = Some(SetupReceipt {
        operation: digest(8),
        action: SetupAction::Enable,
        generation: active.generation(),
    });
    let bytes = record.encode(&binding).unwrap();
    assert!(bytes.len() <= 4096);
    assert_eq!(Record::decode(&bytes, &binding).unwrap(), record);
    let json = String::from_utf8(bytes).unwrap();
    assert!(json.contains("\\u0000"));
    assert!(json.contains("18446744073709551615"));
    assert_eq!(candidate.alias().len(), 117);
    assert_eq!(candidate.ciphertext(), &[4; 256]);
    assert_eq!(candidate.spki_digest(), digest(4));
    assert!(candidate.alias().starts_with("taypeer_biometry_v1_"));
    assert_eq!(binding.epoch(), u64::MAX);
    assert_eq!(binding.working_copy(), digest(1));
    assert_eq!(binding.root(), digest(2));
    assert_eq!(binding.database().as_str().len(), 128);
    assert_eq!(
        binding.namespace(),
        Binding::for_profile(
            profile.path(),
            digest(9),
            DatabaseId::new("PUBLIC other"),
            digest(8),
            0
        )
        .unwrap()
        .namespace()
    );
    let diagnostic = format!("{record:?} {candidate:?}");
    assert!(!diagnostic.contains(candidate.alias()));
    assert!(!diagnostic.contains(&STANDARD.encode(candidate.ciphertext())));
    assert!(!diagnostic.contains(&digest(4).to_string()));
}

#[test]
fn absent_unavailable_and_corrupt_records_are_distinct_and_preserved() {
    let mut fixture = Fixture::new();
    assert_eq!(fixture.registry.confirmed(), Ok(None));
    assert_eq!(
        fixture.registry.retry(digest(8)),
        Err(RegistryError::NoPending)
    );
    for bytes in [
        vec![b' '; 4097],
        b"{\"version\":1".to_vec(),
        b"null".to_vec(),
    ] {
        fs::write(&fixture.registry.path, &bytes).unwrap();
        assert_eq!(fixture.registry.confirmed(), Err(RegistryError::Invalid));
        assert_eq!(
            fixture
                .registry
                .begin_enable(public_proof(&fixture.binding, 3, 8)),
            Err(RegistryError::Invalid)
        );
        assert_eq!(fixture.bytes(), bytes);
    }
    fs::remove_file(&fixture.registry.path).unwrap();
    fs::create_dir(&fixture.registry.path).unwrap();
    assert_eq!(fixture.registry.confirmed(), Err(RegistryError::Io));
    assert!(fixture.registry.path.is_dir());
}

#[test]
fn strict_schema_alias_encoding_and_fixed_fields_refuse_without_replacement() {
    let mut fixture = Fixture::new();
    let active = fixture.enable(3, 8);
    let valid = String::from_utf8(fixture.bytes()).unwrap();
    let cipher = STANDARD.encode(active.ciphertext());
    let mut trailing_bits = cipher.clone().into_bytes();
    trailing_bits[341] = if trailing_bits[341] == b'A' {
        b'B'
    } else {
        trailing_bits[341] + 1
    };
    let altered = [
        valid.replacen("\"version\":1", "\"version\":2", 1),
        valid.replacen("\"scheme\":", "\"unknown\":0,\"scheme\":", 1),
        valid.replacen("rsa_oaep_sha256_mgf1_sha1", "future_scheme", 1),
        valid.replacen(active.alias(), "taypeer_biometry_v1_wrong", 1),
        valid.replacen(&cipher, &cipher[..343], 1),
        valid.replacen(&cipher, &String::from_utf8(trailing_bits).unwrap(), 1),
        valid.replacen(&cipher, &format!("-{}", &cipher[1..]), 1),
        valid.replacen("PUBLIC database", "", 1),
        valid.replacen("PUBLIC database", &"x".repeat(129), 1),
        valid.replacen("\"generation\":", "\"unknown\":true,\"generation\":", 1),
        valid.replacen(&digest(1).to_string(), &"A".repeat(64), 1),
        valid.replacen(&digest(8).to_string(), &"A".repeat(64), 1),
        valid.replacen("\"receipt\":{", "\"receipt\":null,\"extra\":{", 1),
        format!(" {valid}"),
    ];
    for bytes in altered {
        fs::write(&fixture.registry.path, bytes.as_bytes()).unwrap();
        assert_eq!(fixture.registry.confirmed(), Err(RegistryError::Invalid));
        assert_eq!(fixture.bytes(), bytes.as_bytes());
    }
    assert_eq!(
        Generation::parse(&"A".repeat(32)),
        Err(RegistryError::Invalid)
    );
    assert_eq!(
        Generation::parse(&"0".repeat(31)),
        Err(RegistryError::Invalid)
    );
}

#[test]
fn schema_rejects_unproven_stage_inconsistent_receipt_and_foreign_copy() {
    let mut fixture = Fixture::new();
    fixture
        .registry
        .begin_enable(public_proof(&fixture.binding, 3, 8))
        .unwrap();
    let valid = String::from_utf8(fixture.bytes()).unwrap();
    for invalid in [
        valid.replace("proof_confirmed", "prompt_succeeded"),
        valid.replacen(&digest(1).to_string(), &digest(7).to_string(), 1),
    ] {
        fs::write(&fixture.registry.path, invalid.as_bytes()).unwrap();
        assert_eq!(fixture.registry.confirmed(), Err(RegistryError::Invalid));
        assert_eq!(fixture.bytes(), invalid.as_bytes());
    }
    fs::write(&fixture.registry.path, valid.as_bytes()).unwrap();
    fixture.registry.finish_enable(digest(8)).unwrap();
    let invalid_receipt = String::from_utf8(fixture.bytes()).unwrap().replacen(
        "\"action\":\"enable\"",
        "\"action\":\"disable\"",
        1,
    );
    fs::write(&fixture.registry.path, invalid_receipt.as_bytes()).unwrap();
    assert_eq!(fixture.registry.confirmed(), Err(RegistryError::Invalid));
    assert_eq!(fixture.bytes(), invalid_receipt.as_bytes());
}

#[test]
fn locator_is_profile_copy_database_root_bound_but_epoch_independent() {
    let mut fixture = Fixture::new();
    let active = fixture.enable(3, 8);
    let newer = binding(fixture.profile.path(), "PUBLIC database", 2);
    let new_registry = Registry::new(fixture.profile.path(), newer.clone()).unwrap();
    assert_eq!(new_registry.path, fixture.registry.path);
    let next = wrapper(&newer, 3);
    assert_ne!(next.alias(), active.alias());
    let other_profile = tempfile::tempdir().unwrap();
    assert!(matches!(
        Registry::new(other_profile.path(), fixture.binding.clone()),
        Err(RegistryError::Invalid)
    ));
    for foreign in [
        Binding::for_profile(
            fixture.profile.path(),
            digest(6),
            DatabaseId::new("PUBLIC database"),
            digest(2),
            1,
        )
        .unwrap(),
        Binding::for_profile(
            fixture.profile.path(),
            digest(1),
            DatabaseId::new("PUBLIC other"),
            digest(2),
            1,
        )
        .unwrap(),
        Binding::for_profile(
            fixture.profile.path(),
            digest(1),
            DatabaseId::new("PUBLIC database"),
            digest(6),
            1,
        )
        .unwrap(),
    ] {
        assert_ne!(
            Registry::new(fixture.profile.path(), foreign).unwrap().path,
            fixture.registry.path
        );
    }
}

#[test]
fn actual_prepublication_failure_preserves_active_and_retries_exact_candidate() {
    let mut fixture = Fixture::new();
    let active = fixture.enable(3, 8);
    let before = fixture.bytes();
    let probe = PublicationProbe::install(&mut fixture);
    probe.fail_next_before_rename();
    assert_eq!(
        fixture
            .registry
            .begin_enable(public_proof(&fixture.binding, 4, 9)),
        Err(RegistryError::Io)
    );
    assert_eq!(fixture.bytes(), before);
    assert_eq!(
        fs::read(probe.blocked.join("PUBLIC retained")).unwrap(),
        b"PUBLIC original"
    );
    assert_eq!(
        fixture
            .registry
            .begin_disable(active.generation(), digest(10)),
        Err(RegistryError::Conflict)
    );
    assert_eq!(
        fixture.registry.retry(digest(10)),
        Err(RegistryError::Stale)
    );
    assert_eq!(
        fixture.registry.retry(digest(9)),
        Ok(SetupState::PendingEnable {
            operation: digest(9),
            candidate: wrapper(&fixture.binding, 4),
            active: Some(active),
        })
    );
    let attempts = probe.exact.lock().unwrap();
    assert_eq!(attempts.len(), 2);
    assert_eq!(attempts[0], attempts[1]);
    drop(attempts);
    assert_eq!(
        fixture.registry.finish_enable(digest(9)),
        Ok(SetupState::Enabled(wrapper(&fixture.binding, 4)))
    );
}

#[test]
fn lost_pending_reply_keeps_exact_intent_and_active_until_final_ack() {
    let mut fixture = Fixture::new();
    let active = fixture.enable(3, 8);
    let probe = PublicationProbe::install(&mut fixture);
    probe.lose_next();
    assert_eq!(
        fixture
            .registry
            .begin_enable(public_proof(&fixture.binding, 4, 9)),
        Err(RegistryError::CommitUncertain)
    );
    let pending = fixture.bytes();
    let loaded = load(&fixture.registry.path, &fixture.binding)
        .unwrap()
        .unwrap();
    assert_eq!(loaded.record.active, Some(active.clone()));
    assert_eq!(fixture.registry.confirmed(), Err(RegistryError::Conflict));
    assert_eq!(
        fixture.registry.finish_enable(digest(9)),
        Err(RegistryError::Conflict)
    );
    assert_eq!(
        fixture.registry.retry(digest(9)),
        Ok(SetupState::PendingEnable {
            operation: digest(9),
            candidate: wrapper(&fixture.binding, 4),
            active: Some(active),
        })
    );
    assert_eq!(fixture.bytes(), pending);
    let attempts = probe.exact.lock().unwrap();
    assert_eq!(attempts[0], attempts[1]);
    drop(attempts);
    fixture.registry.finish_enable(digest(9)).unwrap();
}

#[test]
fn cold_final_receipt_retry_and_initial_status_republish_equal_bytes() {
    let mut fixture = Fixture::new();
    let probe = PublicationProbe::install(&mut fixture);
    fixture
        .registry
        .begin_enable(public_proof(&fixture.binding, 3, 8))
        .unwrap();
    probe.lose_next();
    assert_eq!(
        fixture.registry.finish_enable(digest(8)),
        Err(RegistryError::CommitUncertain)
    );
    let exact_final = fixture.bytes();
    fixture.registry = Registry::new(fixture.profile.path(), fixture.binding.clone()).unwrap();
    fixture.registry.publisher = Box::new(probe.clone());
    let calls = probe.count();
    assert_eq!(
        fixture.registry.retry(digest(8)),
        Ok(SetupState::Enabled(wrapper(&fixture.binding, 3)))
    );
    assert_eq!(probe.count(), calls + 1);
    assert_eq!(fixture.bytes(), exact_final);
    probe.lose_next();
    assert_eq!(
        fixture.registry.confirmed(),
        Err(RegistryError::CommitUncertain)
    );
    assert_eq!(fixture.bytes(), exact_final);
    assert_eq!(
        fixture.registry.retry(digest(8)),
        Ok(SetupState::Enabled(wrapper(&fixture.binding, 3)))
    );
    assert_eq!(
        fixture.registry.confirmed(),
        Ok(Some(SetupState::Enabled(wrapper(&fixture.binding, 3))))
    );
    assert_eq!(fixture.bytes(), exact_final);
}

#[test]
fn enable_disable_reenroll_refuses_old_operations_and_generations() {
    let mut fixture = Fixture::new();
    let old = fixture.enable(3, 8);
    assert_eq!(
        fixture.registry.begin_disable(old.generation(), digest(9)),
        Ok(SetupState::PendingDisable {
            operation: digest(9),
            active: old.clone(),
        })
    );
    let pending = fixture.bytes();
    assert_eq!(fixture.registry.retry(digest(8)), Err(RegistryError::Stale));
    assert_eq!(fixture.bytes(), pending);
    fixture
        .registry
        .finish_disable(old.generation(), digest(9))
        .unwrap();
    let newer = fixture.enable(4, 10);
    let stable = fixture.bytes();
    for operation in [digest(8), digest(9)] {
        assert_eq!(fixture.registry.retry(operation), Err(RegistryError::Stale));
    }
    assert_eq!(
        fixture.registry.finish_enable(digest(8)),
        Err(RegistryError::Stale)
    );
    assert_eq!(
        fixture.registry.finish_disable(old.generation(), digest(9)),
        Err(RegistryError::Stale)
    );
    assert_eq!(
        fixture.registry.begin_disable(old.generation(), digest(11)),
        Err(RegistryError::Stale)
    );
    assert_eq!(
        fixture
            .registry
            .begin_disable(newer.generation(), digest(10)),
        Err(RegistryError::Stale)
    );
    assert_eq!(
        fixture
            .registry
            .begin_enable(public_proof(&fixture.binding, 5, 10)),
        Err(RegistryError::Stale)
    );
    assert_eq!(fixture.bytes(), stable);
}

#[test]
fn lost_disable_final_reply_cold_retry_confirms_same_generation_only() {
    let mut fixture = Fixture::new();
    let active = fixture.enable(3, 8);
    fixture
        .registry
        .begin_disable(active.generation(), digest(9))
        .unwrap();
    let probe = PublicationProbe::install(&mut fixture);
    probe.lose_next();
    assert_eq!(
        fixture
            .registry
            .finish_disable(active.generation(), digest(9)),
        Err(RegistryError::CommitUncertain)
    );
    let disabled = fixture.bytes();
    fixture.registry = Registry::new(fixture.profile.path(), fixture.binding.clone()).unwrap();
    fixture.registry.publisher = Box::new(probe.clone());
    assert_eq!(
        fixture.registry.finish_disable(generation(4), digest(9)),
        Err(RegistryError::Stale)
    );
    assert_eq!(fixture.bytes(), disabled);
    assert_eq!(probe.count(), 1);
    assert_eq!(
        fixture
            .registry
            .finish_disable(active.generation(), digest(9)),
        Ok(SetupState::Disabled)
    );
    assert_eq!(probe.count(), 2);
    assert_eq!(fixture.bytes(), disabled);
}

#[test]
fn stale_epoch_proof_and_cold_pending_completion_cannot_enable_current_epoch() {
    let mut fixture = Fixture::new();
    let newer = binding(fixture.profile.path(), "PUBLIC database", 2);
    assert_eq!(
        fixture.registry.begin_enable(public_proof(&newer, 3, 8)),
        Err(RegistryError::Stale)
    );
    assert!(!fixture.registry.path.exists());
    fixture
        .registry
        .begin_enable(public_proof(&fixture.binding, 3, 8))
        .unwrap();
    let pending = fixture.bytes();
    fixture.registry = Registry::new(fixture.profile.path(), newer).unwrap();
    assert_eq!(
        fixture.registry.finish_enable(digest(8)),
        Err(RegistryError::Stale)
    );
    assert_eq!(fixture.bytes(), pending);
}

#[test]
fn lost_enable_survives_rotation_exact_metadata_disposition_and_cold_retry_then_reenroll() {
    let mut fixture = Fixture::new();
    let old = wrapper(&fixture.binding, 3);
    let probe = PublicationProbe::install(&mut fixture);
    probe.lose_next();
    assert_eq!(
        fixture
            .registry
            .begin_enable(public_proof(&fixture.binding, 3, 8)),
        Err(RegistryError::CommitUncertain)
    );
    let pending = fixture.bytes();
    let current = binding(fixture.profile.path(), "PUBLIC database", 2);
    fixture.registry.current_binding(current.clone()).unwrap();
    assert_eq!(fixture.registry.retry(digest(9)), Err(RegistryError::Stale));
    assert_eq!(
        fixture.registry.retry(digest(8)),
        Ok(SetupState::PendingEnable {
            operation: digest(8),
            candidate: old.clone(),
            active: None
        })
    );
    assert_eq!(fixture.bytes(), pending);
    {
        let attempts = probe.exact.lock().unwrap();
        assert_eq!(
            attempts[0], attempts[1],
            "Rotation must not replace frozen publication bytes"
        );
    }
    assert_eq!(
        fixture.registry.finish_enable(digest(8)),
        Err(RegistryError::Stale)
    );
    assert_eq!(fixture.bytes(), pending);
    probe.lose_next();
    assert_eq!(
        fixture.registry.finish_earlier_epoch_enable(digest(8)),
        Err(RegistryError::CommitUncertain)
    );
    let final_bytes = fixture.bytes();
    let stored = Record::decode(&final_bytes, &current).unwrap();
    assert_eq!(stored.active, Some(old.clone()));
    assert!(stored.pending.is_none());
    assert_eq!(stored.receipt.unwrap().operation, digest(8));
    assert_eq!(old.binding().epoch(), 1);
    assert_eq!(old.ciphertext(), &[3; 256]);
    fixture.registry = Registry::new(fixture.profile.path(), current.clone()).unwrap();
    fixture.registry.publisher = Box::new(probe.clone());
    let calls = probe.count();
    assert_eq!(
        fixture.registry.retry(digest(8)),
        Ok(SetupState::Enabled(old.clone()))
    );
    assert_eq!(
        probe.count(),
        calls + 1,
        "Equal cold bytes must be re-published durably"
    );
    assert_eq!(fixture.bytes(), final_bytes);
    fixture
        .registry
        .begin_disable(old.generation(), digest(9))
        .unwrap();
    fixture
        .registry
        .finish_disable(old.generation(), digest(9))
        .unwrap();
    fixture
        .registry
        .begin_enable(public_proof(&current, 4, 10))
        .unwrap();
    assert_eq!(
        fixture.registry.finish_enable(digest(10)),
        Ok(SetupState::Enabled(wrapper(&current, 4)))
    );
    let newer = fixture.bytes();
    assert_eq!(fixture.registry.retry(digest(8)), Err(RegistryError::Stale));
    assert_eq!(
        fixture.registry.begin_disable(old.generation(), digest(11)),
        Err(RegistryError::Stale)
    );
    assert_eq!(fixture.bytes(), newer);
}

#[test]
fn metadata_only_disposition_refuses_foreign_permanent_scope_and_future_epoch() {
    let mut fixture = Fixture::new();
    fixture
        .registry
        .begin_enable(public_proof(&fixture.binding, 3, 8))
        .unwrap();
    let before = fixture.bytes();
    let foreign = Binding::for_profile(
        fixture.profile.path(),
        digest(9),
        fixture.binding.database().clone(),
        fixture.binding.root(),
        2,
    )
    .unwrap();
    assert_eq!(
        fixture.registry.current_binding(foreign),
        Err(RegistryError::Stale)
    );
    fixture
        .registry
        .current_binding(binding(fixture.profile.path(), "PUBLIC database", 0))
        .unwrap();
    assert_eq!(
        fixture.registry.finish_earlier_epoch_enable(digest(8)),
        Err(RegistryError::Stale)
    );
    assert_eq!(fixture.bytes(), before);
    fixture
        .registry
        .current_binding(binding(fixture.profile.path(), "PUBLIC database", 2))
        .unwrap();
    assert_eq!(
        fixture.registry.finish_earlier_epoch_enable(digest(9)),
        Err(RegistryError::Stale)
    );
    assert_eq!(fixture.bytes(), before);
}

#[test]
fn unknown_publication_cannot_overwrite_a_different_valid_actual_record() {
    let mut fixture = Fixture::new();
    fixture.enable(3, 8);
    let original = fixture.bytes();
    let probe = PublicationProbe::install(&mut fixture);
    probe.lose_next();
    assert_eq!(
        fixture
            .registry
            .begin_enable(public_proof(&fixture.binding, 4, 9)),
        Err(RegistryError::CommitUncertain)
    );
    // Simulate replacement by a different valid operation under this same copy.
    let mut other = Record::decode(&original, &fixture.binding).unwrap();
    other.pending = Some(PendingSetup::enable(
        digest(10),
        wrapper(&fixture.binding, 5),
    ));
    let actual = other.encode(&fixture.binding).unwrap();
    fs::write(&fixture.registry.path, &actual).unwrap();
    assert_eq!(fixture.registry.retry(digest(9)), Err(RegistryError::Stale));
    assert_eq!(fixture.bytes(), actual);
    assert_eq!(probe.count(), 1);
}
