//! PUBLIC Native coordination tests. No fake port can authenticate a worker.
use super::*;
use crate::{BiometricPromptCall, BiometricPromptOutcome, CredentialPort, Host};
use std::sync::atomic::{AtomicUsize, Ordering};

type CredentialAddress = (String, String);
#[derive(Default)]
struct Credentials(Mutex<BTreeMap<CredentialAddress, Vec<u8>>>);
impl CredentialPort for Credentials {
    fn get(&self, service: String, account: String) -> Result<Option<Vec<u8>>, AndroidError> {
        Ok(self.0.lock().unwrap().get(&(service, account)).cloned())
    }
    fn set(&self, service: String, account: String, bytes: Vec<u8>) -> Result<(), AndroidError> {
        self.0.lock().unwrap().insert((service, account), bytes);
        Ok(())
    }
}
#[derive(Default)]
struct RefusingPort {
    creates: AtomicUsize,
    resolves: AtomicUsize,
    prompts: AtomicUsize,
}
impl OsBiometryPort for RefusingPort {
    fn availability(&self) -> Result<BiometricAvailability, AndroidError> {
        Ok(BiometricAvailability::NoHardware)
    }
    fn create_key(
        &self,
        _: BiometricBindingView,
        _: String,
    ) -> Result<OsBiometricKeyLease, AndroidError> {
        self.creates.fetch_add(1, Ordering::Relaxed);
        Err(refusal(BiometryFailure::Unavailable))
    }
    fn resolve_key(
        &self,
        _: BiometricBindingView,
        _: String,
        _: String,
        _: String,
        _: String,
    ) -> Result<OsBiometricKeyLease, AndroidError> {
        self.resolves.fetch_add(1, Ordering::Relaxed);
        Err(refusal(BiometryFailure::Unavailable))
    }
    fn run_prompt(
        &self,
        _: Arc<BiometricPromptCall>,
    ) -> Result<BiometricPromptOutcome, AndroidError> {
        self.prompts.fetch_add(1, Ordering::Relaxed);
        Err(refusal(BiometryFailure::Unavailable))
    }
    fn retire_lease(&self, _: String) {}
    fn delete_key(
        &self,
        _: BiometricBindingView,
        _: String,
        _: String,
    ) -> Result<(), AndroidError> {
        Ok(())
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    broker: Arc<Broker>,
    port: Arc<RefusingPort>,
    binding: Binding,
}
impl Fixture {
    fn registered() -> Self {
        let mut fixture = Self::new();
        let path = fixture
            ._directory
            .path()
            .join("PUBLIC biometric rotation.taypeer");
        let writer = fixture
            .broker
            .runtime
            .platform_cipher_writer(&path)
            .unwrap();
        let author = writer.author(None).unwrap();
        let identity =
            taypeer_trust::Identity::new(author.public(), writer.transport_public()).unwrap();
        let seed = DatabaseService::prepare_managed(
            "PUBLIC biometric metadata".into(),
            b"PUBLIC original password",
            &author,
            identity,
            1,
            taypeer_core::DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap(),
        )
        .unwrap();
        writer.create(seed).unwrap();
        fixture.broker.runtime.register_working_copy(&path).unwrap();
        let copy = fixture
            .broker
            .runtime
            .working_copies()
            .unwrap()
            .into_iter()
            .next()
            .unwrap();
        fixture.binding = fixture.broker.current(&copy.into()).unwrap().0;
        fixture
    }
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let host = Host::new(
            directory.path().to_string_lossy().into_owned(),
            Box::new(Credentials::default()),
        )
        .unwrap();
        let port = Arc::new(RefusingPort::default());
        let broker = Broker::new(Arc::clone(&host.runtime), port.clone());
        let binding = Binding::for_profile(
            host.runtime.profile().directory(),
            Digest::of(b"PUBLIC Native copy"),
            taypeer_core::DatabaseId::new("PUBLIC Native database"),
            Digest::of(b"PUBLIC Native root"),
            7,
        )
        .unwrap();
        Self {
            _directory: directory,
            broker,
            port,
            binding,
        }
    }
    fn candidate(&self, generation: &str) -> WrapperRecord {
        WrapperRecord::new(
            self.binding.clone(),
            Generation::parse(generation).unwrap(),
            Digest::of(b"PUBLIC fixture SPKI marker, not OS proof"),
            [0x50; 256],
        )
        .unwrap()
    }
}
#[test]
fn releasing_a_retired_reservation_cannot_release_the_next_native_actor() {
    let fixture = Fixture::new();
    let first = fixture.broker.reserve(&fixture.binding).unwrap();
    assert!(matches!(
        fixture.broker.reserve(&fixture.binding),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Busy
        })
    ));
    first.release();
    let second = fixture.broker.reserve(&fixture.binding).unwrap();
    first.release();
    drop(first);
    second.check().unwrap();
    assert!(matches!(
        fixture.broker.reserve(&fixture.binding),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Busy
        })
    ));
    drop(second);
    fixture
        .broker
        .reserve(&fixture.binding)
        .unwrap()
        .check()
        .unwrap();
}
#[test]
fn stale_generation_never_becomes_a_fresh_orphan_or_overwrites_confirmed_bytes() {
    let fixture = Fixture::new();
    let owned = fixture.broker.reserve(&fixture.binding).unwrap();
    let operation = Digest::of(b"PUBLIC completed setup fixture, no OS auth");
    let candidate = fixture.candidate("11111111111111111111111111111111");
    let mut registry = owned.slot().registry.lock().unwrap();
    registry
        .begin_enable(
            super::super::registry::ConfirmedEnrollment::after_worker_proof(
                operation,
                candidate.clone(),
            ),
        )
        .unwrap();
    registry.finish_enable(operation).unwrap();
    let path = fixture.broker.profile.join(format!(
        "biometry-{}.json",
        fixture.binding.copy_digest().unwrap()
    ));
    let before = std::fs::read(&path).unwrap();
    assert!(matches!(
        registry.require_fresh_generation(candidate.generation()),
        Err(super::super::registry::RegistryError::Stale)
    ));
    registry
        .require_fresh_generation(Generation::parse("22222222222222222222222222222222").unwrap())
        .unwrap();
    assert_eq!(std::fs::read(path).unwrap(), before);
    assert_eq!(fixture.port.creates.load(Ordering::Relaxed), 0);
}
#[test]
fn unavailable_os_cannot_create_a_key_or_turn_registry_metadata_into_enabled() {
    let fixture = Fixture::registered();
    assert!(matches!(
        fixture.broker.require_available(),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Unavailable
        })
    ));
    let owned = fixture.broker.reserve(&fixture.binding).unwrap();
    let operation = Digest::of(b"PUBLIC setup receipt only");
    let state = owned
        .slot()
        .registry
        .lock()
        .unwrap()
        .begin_enable(
            super::super::registry::ConfirmedEnrollment::after_worker_proof(
                operation,
                fixture.candidate("11111111111111111111111111111111"),
            ),
        )
        .unwrap();
    assert!(matches!(
        fixture.broker.finish_setup(&owned, state),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Unavailable
        })
    ));
    let state = owned
        .slot()
        .registry
        .lock()
        .unwrap()
        .retry(operation)
        .unwrap();
    let view = fixture.broker.view(&fixture.binding, Some(state)).unwrap();
    assert_eq!(view.state, super::super::BiometryState::Pending);
    assert_eq!(view.operation, Some(operation.to_string()));
    assert_eq!(fixture.port.creates.load(Ordering::Relaxed), 0);
    assert_eq!(fixture.port.prompts.load(Ordering::Relaxed), 0);
}

#[test]
fn actual_registered_archive_rotation_disposes_old_setup_metadata_without_os_key_or_prompt() {
    let fixture = Fixture::registered();
    let operation = Digest::of(b"PUBLIC proof-confirmed old setup fixture");
    let old = fixture.candidate("11111111111111111111111111111111");
    {
        let owned = fixture.broker.reserve(&fixture.binding).unwrap();
        owned
            .slot()
            .registry
            .lock()
            .unwrap()
            .begin_enable(
                super::super::registry::ConfirmedEnrollment::after_worker_proof(
                    operation,
                    old.clone(),
                ),
            )
            .unwrap();
    }
    let copy = fixture
        .broker
        .runtime
        .working_copies()
        .unwrap()
        .into_iter()
        .next()
        .unwrap();
    let mut service = DatabaseService::new();
    let session = fixture
        .broker
        .runtime
        .open_local(&mut service, &copy.path, b"PUBLIC original password", None)
        .unwrap();
    service
        .rotate_password(
            &session,
            Digest::of(b"PUBLIC actual epoch rotation"),
            b"PUBLIC different password",
            None,
        )
        .unwrap();
    service.lock(&session).unwrap();
    let ciphertext = std::fs::read(&copy.path).unwrap();
    let catalog = fixture.broker.runtime.working_copies().unwrap();
    let (current, managed, _) = fixture.broker.current(&copy.clone().into()).unwrap();
    assert!(current.epoch() > fixture.binding.epoch());
    let owned = fixture.broker.reserve(&current).unwrap();
    let pending = owned
        .slot()
        .registry
        .lock()
        .unwrap()
        .retry(operation)
        .unwrap();
    let state = fixture.broker.finish_setup(&owned, pending).unwrap();
    let view = fixture.broker.view(&current, Some(state)).unwrap();
    assert_eq!(view.state, super::super::BiometryState::NeedsMaster);
    assert_eq!(view.generation, Some(old.generation().to_string()));
    assert!(view.operation.is_none());
    assert_eq!(fixture.port.creates.load(Ordering::Relaxed), 0);
    assert_eq!(fixture.port.resolves.load(Ordering::Relaxed), 0);
    assert_eq!(fixture.port.prompts.load(Ordering::Relaxed), 0);
    assert_eq!(fixture.broker.runtime.working_copies().unwrap(), catalog);
    assert_eq!(std::fs::read(&copy.path).unwrap(), ciphertext);
    assert_eq!(fixture.broker.current(&copy.into()).unwrap().1, managed);
}
#[test]
fn permanent_binding_and_alias_are_checked_before_untrusted_spki_or_prompt() {
    let fixture = Fixture::new();
    let attempt = Digest::of(b"PUBLIC exact attempt");
    let generation = Generation::parse("11111111111111111111111111111111").unwrap();
    let mut lease = OsBiometricKeyLease {
        id: Digest::of(b"PUBLIC lease ID").to_string(),
        generation: generation.to_string(),
        binding: BiometricBindingView::from_binding(&fixture.binding).unwrap(),
        attempt: attempt.to_string(),
        alias: WrapperRecord::scoped_alias(&fixture.binding, generation).unwrap(),
        spki: vec![0; 4097],
    };
    lease.binding.epoch += 1;
    assert!(matches!(
        fixture
            .broker
            .validate_key(&lease, &fixture.binding, attempt),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Stale
        })
    ));
    lease.binding.epoch -= 1;
    lease.alias = "PUBLIC foreign OS alias".into();
    assert!(matches!(
        fixture
            .broker
            .validate_key(&lease, &fixture.binding, attempt),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Stale
        })
    ));
    lease.alias = WrapperRecord::scoped_alias(&fixture.binding, generation).unwrap();
    assert!(
        fixture
            .broker
            .validate_key(&lease, &fixture.binding, attempt)
            .is_err()
    );
    assert_eq!(fixture.port.prompts.load(Ordering::Relaxed), 0);
}
#[test]
fn arbitrary_copy_and_corrupt_registry_are_not_first_setup_or_os_authority() {
    let fixture = Fixture::new();
    let copy = WorkingCopyView {
        database: fixture.binding.database().to_string(),
        root: fixture.binding.root().to_string(),
        path: fixture
            .broker
            .profile
            .join("PUBLIC unregistered.taypeer")
            .to_string_lossy()
            .into_owned(),
    };
    assert!(matches!(
        fixture.broker.current(&copy),
        Err(AndroidError::Biometry {
            failure: BiometryFailure::Stale
        })
    ));
    let path = fixture.broker.profile.join(format!(
        "biometry-{}.json",
        fixture.binding.copy_digest().unwrap()
    ));
    std::fs::write(&path, b"PUBLIC malformed future registry").unwrap();
    let owned = fixture.broker.reserve(&fixture.binding).unwrap();
    assert!(matches!(
        owned.slot().registry.lock().unwrap().confirmed(),
        Err(super::super::registry::RegistryError::Invalid)
    ));
    assert_eq!(
        std::fs::read(path).unwrap(),
        b"PUBLIC malformed future registry"
    );
    assert_eq!(fixture.port.creates.load(Ordering::Relaxed), 0);
}
