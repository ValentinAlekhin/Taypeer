//! PUBLIC protected receipts exercise real profile serialization and exact publication ACKs.
use super::*;
use crate::profile::{CredentialStore, NativeProfile, ProfileError, Registration};
use std::sync::atomic::{AtomicU8, AtomicUsize, Ordering};
use zeroize::Zeroizing;

type Values = BTreeMap<(String, String), Zeroizing<Vec<u8>>>;
#[derive(Default)]
struct Credentials {
    values: Mutex<Values>,
    fault: AtomicU8,
    writes: AtomicUsize,
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
            .cloned())
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        let handoff = account.starts_with("state:management:");
        if handoff {
            self.writes.fetch_add(1, Ordering::Relaxed);
            if self.fault.load(Ordering::Relaxed) == 1 {
                return Err(ProfileError::CommitUncertain);
            }
        }
        self.values.lock().unwrap().insert(
            (service.into(), account.into()),
            Zeroizing::new(bytes.to_vec()),
        );
        if handoff && self.fault.load(Ordering::Relaxed) == 2 {
            return Err(ProfileError::CommitUncertain);
        }
        Ok(())
    }
}
fn registration() -> Registration {
    Registration {
        database: DatabaseId::new("PUBLIC management database"),
        root: Digest::of(b"PUBLIC management root"),
        working_copy: Digest::of(b"PUBLIC management working copy"),
    }
}
fn intent(registration: &Registration) -> ManagementIntent {
    let manager = taypeer_trust::AuthorKey::generate().unwrap();
    let recipient = taypeer_trust::AuthorKey::generate().unwrap();
    ManagementIntent {
        database: registration.database.clone(),
        root: registration.root,
        previous: Digest::of(b"PUBLIC management previous"),
        operation: Digest::of(b"PUBLIC management operation"),
        manager: manager.device_id(),
        recipient: recipient.device_id(),
    }
}
#[test]
fn uncertain_decision_before_and_after_publication_requires_republish_ack_and_exact_intent() {
    for fault in [1, 2] {
        let directory = tempfile::tempdir().unwrap();
        let credentials = Arc::new(Credentials::default());
        let lease = NativeProfile::acquire_platform(directory.path(), credentials.clone()).unwrap();
        let profile = lease.profile();
        let registration = registration();
        let intent = intent(&registration);
        let store = Store::default();
        credentials.fault.store(fault, Ordering::Relaxed);
        let record = Record {
            intent: intent.clone(),
            direction: Direction::Recipient,
            phase: Phase::AcceptRequested,
        };
        assert!(matches!(
            store.update(profile, &registration, &intent, |_| Ok(record.clone())),
            Err(RuntimeError::Service(
                taypeer_services::ServiceError::Storage(taypeer_storage::Error::CommitUncertain)
            ))
        ));
        if fault == 2 {
            let cold = Store::default();
            let writes = credentials.writes.load(Ordering::Relaxed);
            assert!(matches!(
                cold.records(profile, &registration),
                Err(RuntimeError::Service(
                    taypeer_services::ServiceError::Storage(
                        taypeer_storage::Error::CommitUncertain
                    )
                ))
            ));
            assert!(
                credentials.writes.load(Ordering::Relaxed) > writes,
                "a cold equal read must confirm publication before reporting a durable decision"
            );
        }
        let first_writes = credentials.writes.load(Ordering::Relaxed);
        assert!(store.record(profile, &registration, &intent, true).is_err());
        assert!(
            credentials.writes.load(Ordering::Relaxed) > first_writes,
            "equal read is not a durable ACK"
        );
        credentials.fault.store(0, Ordering::Relaxed);
        assert!(
            store
                .record(profile, &registration, &intent, true)
                .unwrap()
                .unwrap()
                .phase
                == Phase::AcceptRequested
        );
        let mut other = intent.clone();
        other.previous = Digest::of(b"PUBLIC changed previous");
        let before = credentials.values.lock().unwrap().clone();
        assert!(matches!(
            store.update(profile, &registration, &other, |_| unreachable!()),
            Err(RuntimeError::Management(ManagementFailure::Conflict))
        ));
        assert!(*credentials.values.lock().unwrap() == before);
        // A new owner reads the same confirmed decision, without signing or inventing a new intent.
        assert!(
            Store::default().records(profile, &registration).unwrap()[0].phase
                == Phase::AcceptRequested
        );
        assert_eq!(
            Store::default().records(profile, &registration).unwrap()[0]
                .view()
                .phase,
            ManagementPhase::AcceptRequested
        );
    }
}
#[test]
fn corrupt_and_future_handoff_receipts_are_preserved_without_fallback_or_overwrite() {
    for value in [
        serde_json::json!({"version":99}),
        serde_json::json!({"PUBLIC":"corrupt"}),
    ] {
        let directory = tempfile::tempdir().unwrap();
        let credentials = Arc::new(Credentials::default());
        let lease = NativeProfile::acquire_platform(directory.path(), credentials.clone()).unwrap();
        let profile = lease.profile();
        let registration = registration();
        let key = format!(
            "management:{}:{}",
            registration.database, registration.working_copy
        );
        profile.save_state(&key, &value).unwrap();
        let before = credentials.values.lock().unwrap().clone();
        let store = Store::default();
        assert!(matches!(
            store.records(profile, &registration),
            Err(RuntimeError::Profile(ProfileError::Invalid))
        ));
        let intent = intent(&registration);
        assert!(
            store
                .update(profile, &registration, &intent, |_| unreachable!())
                .is_err()
        );
        assert!(*credentials.values.lock().unwrap() == before);
    }
}
