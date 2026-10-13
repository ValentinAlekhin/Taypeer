//! PUBLIC access metadata exercises the real profile namespace and durable owner.
use super::*;
use crate::{RuntimeHost, profile::CredentialStore, session::SessionController};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::atomic::{AtomicI64, AtomicU8, AtomicUsize, Ordering},
};
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
        if account.starts_with("state:entry-access:") && self.fault.load(Ordering::Relaxed) == 3 {
            return Err(ProfileError::Credentials);
        }
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned())
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        let access = account.starts_with("state:entry-access:");
        if access {
            self.writes.fetch_add(1, Ordering::Relaxed);
            if self.fault.load(Ordering::Relaxed) == 1 {
                return Err(ProfileError::Io);
            }
        }
        self.values.lock().unwrap().insert(
            (service.into(), account.into()),
            Zeroizing::new(bytes.to_vec()),
        );
        if access && self.fault.load(Ordering::Relaxed) == 2 {
            return Err(ProfileError::CommitUncertain);
        }
        Ok(())
    }
}
struct Clock(AtomicI64);
impl AccessClock for Clock {
    fn utc_millis(&self) -> Result<i64, ProfileError> {
        Ok(self.0.load(Ordering::Relaxed))
    }
}
struct Fixture {
    host: RuntimeHost,
    credentials: Arc<Credentials>,
    clock: Arc<Clock>,
    database: DatabaseId,
    entry: EntryId,
}
impl Fixture {
    fn new(directory: &Path, credentials: Arc<Credentials>) -> Self {
        let clock = Arc::new(Clock(AtomicI64::new(1_900_000_000_123)));
        let host = RuntimeHost::with_platform_credentials_and_access_clock(
            directory,
            SessionController::new(Default::default()),
            credentials.clone(),
            clock.clone(),
        )
        .unwrap();
        Self {
            host,
            credentials,
            clock,
            database: DatabaseId::new("PUBLIC database"),
            entry: EntryId::new("PUBLIC entry"),
        }
    }
    fn grant(&self) -> SecretAccessGrant {
        self.host
            .secret_access_grant(self.database.clone(), self.entry.clone())
            .unwrap()
    }
    fn mark(&self) -> Result<Option<i64>, crate::RuntimeError> {
        self.host.accessed_at(&self.database, &self.entry)
    }
    fn record(&self) -> AccessRecord {
        self.host
            .profile()
            .load_state(&AccessRecord::key(&self.database, &self.entry))
            .unwrap()
            .unwrap()
    }
}

#[test]
fn access_is_explicit_exact_utc_and_older_replay_never_rolls_back_backwards_time() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), Arc::new(Credentials::default()));
    let older = fixture.grant();
    assert_eq!(fixture.mark().unwrap(), None);
    assert_eq!(fixture.credentials.writes.load(Ordering::Relaxed), 0);
    assert_eq!(older.confirm().unwrap(), 1_900_000_000_123);
    let original = fixture.record();
    fixture.clock.0.store(-1_234, Ordering::Relaxed);
    assert_eq!(older.confirm().unwrap(), 1_900_000_000_123);
    assert!(fixture.record() == original);
    let newer = fixture.grant();
    assert_eq!(newer.confirm().unwrap(), -1_234);
    let latest = fixture.record();
    assert!(latest.order > original.order && latest.event != original.event);
    assert_eq!(older.confirm().unwrap(), -1_234);
    assert!(fixture.record() == latest);
    assert_eq!(fixture.mark().unwrap(), Some(-1_234));
}

#[test]
fn prepublication_failure_freezes_the_event_and_reserves_order_before_a_newer_action() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), Arc::new(Credentials::default()));
    let failed = fixture.grant();
    fixture.credentials.fault.store(1, Ordering::Relaxed);
    assert_eq!(failed.confirm().unwrap_err(), ProfileError::Io);
    assert_eq!(fixture.mark().unwrap(), None);
    let (frozen_id, frozen_order) = {
        let frozen = failed.event.lock().unwrap();
        let frozen = frozen.as_ref().unwrap();
        (frozen.event, frozen.durable_order.unwrap())
    };
    fixture.clock.0.store(17, Ordering::Relaxed);
    fixture.credentials.fault.store(0, Ordering::Relaxed);
    assert_eq!(failed.confirm().unwrap(), 1_900_000_000_123);
    assert_eq!(fixture.record().event, frozen_id);
    assert_eq!(fixture.record().order, frozen_order);
    let second_failed = fixture.grant();
    fixture.credentials.fault.store(1, Ordering::Relaxed);
    assert_eq!(second_failed.confirm().unwrap_err(), ProfileError::Io);
    fixture.credentials.fault.store(0, Ordering::Relaxed);
    fixture.clock.0.store(3, Ordering::Relaxed);
    assert_eq!(fixture.grant().confirm().unwrap(), 3);
    let latest = fixture.record();
    assert_eq!(second_failed.confirm().unwrap(), 3);
    assert!(fixture.record() == latest);
}

#[test]
fn unknown_publication_reconciles_before_new_events_and_exact_readback_is_not_ack() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), Arc::new(Credentials::default()));
    let grant = fixture.grant();
    fixture.credentials.fault.store(2, Ordering::Relaxed);
    assert_eq!(grant.confirm().unwrap_err(), ProfileError::CommitUncertain);
    let frozen = fixture.record();
    assert!(matches!(
        fixture.mark(),
        Err(crate::RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::CommitUncertain)
        ))
    ));
    fixture.clock.0.store(23, Ordering::Relaxed);
    let newer = fixture.grant();
    // The accepted time is frozen now, but pending publication is confirmed before new write.
    assert_eq!(newer.confirm().unwrap_err(), ProfileError::CommitUncertain);
    assert_eq!(newer.event.lock().unwrap().as_ref().unwrap().utc_ms, 23);
    assert!(
        newer
            .event
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .durable_order
            .is_none()
    );
    assert!(fixture.record() == frozen);
    fixture.credentials.fault.store(1, Ordering::Relaxed);
    assert_eq!(grant.confirm().unwrap_err(), ProfileError::Io);
    assert!(fixture.record() == frozen);
    fixture.credentials.fault.store(0, Ordering::Relaxed);
    assert_eq!(grant.confirm().unwrap(), 1_900_000_000_123);
    assert!(fixture.record() == frozen);
    fixture.clock.0.store(7, Ordering::Relaxed);
    assert_eq!(newer.confirm().unwrap(), 23);
    assert_eq!(grant.confirm().unwrap(), 23);
}

#[test]
fn first_read_failure_retains_the_initial_accepted_utc_for_exact_retry() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), Arc::new(Credentials::default()));
    let grant = fixture.grant();
    fixture.credentials.fault.store(3, Ordering::Relaxed);
    assert_eq!(grant.confirm().unwrap_err(), ProfileError::Credentials);
    assert_eq!(fixture.credentials.writes.load(Ordering::Relaxed), 0);
    let event_id = grant.event.lock().unwrap().as_ref().unwrap().event;
    fixture.clock.0.store(1_900_000_900_123, Ordering::Relaxed);
    fixture.credentials.fault.store(0, Ordering::Relaxed);
    assert_eq!(grant.confirm().unwrap(), 1_900_000_000_123);
    assert_eq!(fixture.record().event, event_id);
    assert_eq!(fixture.mark().unwrap(), Some(1_900_000_000_123));
}

#[test]
fn first_read_failure_old_retry_cannot_replace_a_newer_accepted_backwards_utc() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), Arc::new(Credentials::default()));
    let older = fixture.grant();
    fixture.credentials.fault.store(3, Ordering::Relaxed);
    assert_eq!(older.confirm().unwrap_err(), ProfileError::Credentials);
    assert!(
        older
            .event
            .lock()
            .unwrap()
            .as_ref()
            .unwrap()
            .durable_order
            .is_none()
    );
    fixture.credentials.fault.store(0, Ordering::Relaxed);
    fixture.clock.0.store(-3_000, Ordering::Relaxed);
    let newer = fixture.grant();
    assert_eq!(newer.confirm().unwrap(), -3_000);
    let confirmed = fixture.record();
    fixture.clock.0.store(1_900_999_999_999, Ordering::Relaxed);
    assert_eq!(older.confirm().unwrap(), -3_000);
    assert!(fixture.record() == confirmed);
    assert_eq!(fixture.mark().unwrap(), Some(-3_000));
}

#[test]
fn profile_database_and_entry_are_isolated_and_marks_survive_host_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let credentials = Arc::new(Credentials::default());
    let fixture = Fixture::new(directory.path(), Arc::clone(&credentials));
    fixture.grant().confirm().unwrap();
    let database = fixture.database.clone();
    let entry = fixture.entry.clone();
    assert_eq!(
        fixture
            .host
            .accessed_at(&DatabaseId::new("PUBLIC other db"), &entry)
            .unwrap(),
        None
    );
    assert_eq!(
        fixture
            .host
            .accessed_at(&database, &EntryId::new("PUBLIC other entry"))
            .unwrap(),
        None
    );
    assert_eq!(
        Fixture::new(other.path(), Arc::clone(&credentials))
            .mark()
            .unwrap(),
        None
    );
    let record = fixture.record();
    drop(fixture);
    let reopened = Fixture::new(directory.path(), credentials);
    assert_eq!(reopened.mark().unwrap(), Some(1_900_000_000_123));
    reopened.clock.0.store(2, Ordering::Relaxed);
    reopened.grant().confirm().unwrap();
    assert!(reopened.record().order > record.order);
}

#[test]
fn missing_is_not_corrupt_future_cross_address_or_oversized_state() {
    for variant in 0..5 {
        let directory = tempfile::tempdir().unwrap();
        let fixture = Fixture::new(directory.path(), Arc::new(Credentials::default()));
        fixture.grant().confirm().unwrap();
        let mut record = fixture.record();
        let malformed = match variant {
            0 => b"PUBLIC malformed".to_vec(),
            1 => {
                record.version = 99;
                serde_json::to_vec(&record).unwrap()
            }
            2 => {
                record.order = 0;
                serde_json::to_vec(&record).unwrap()
            }
            3 => {
                record.entry = EntryId::new("PUBLIC different entry");
                serde_json::to_vec(&record).unwrap()
            }
            _ => vec![0; 65_537],
        };
        let address = (
            fixture.host.profile().service(),
            format!(
                "state:{}",
                AccessRecord::key(&fixture.database, &fixture.entry)
            ),
        );
        fixture
            .credentials
            .values
            .lock()
            .unwrap()
            .insert(address.clone(), Zeroizing::new(malformed.clone()));
        assert!(matches!(
            fixture.mark(),
            Err(crate::RuntimeError::Profile(ProfileError::Invalid))
        ));
        assert_eq!(
            fixture.grant().confirm().unwrap_err(),
            ProfileError::Invalid
        );
        assert_eq!(
            &**fixture
                .credentials
                .values
                .lock()
                .unwrap()
                .get(&address)
                .unwrap(),
            malformed.as_slice()
        );
    }
}

#[test]
fn accepted_permission_confirms_after_background_lock_without_reviving_access() {
    let directory = tempfile::tempdir().unwrap();
    let fixture = Fixture::new(directory.path(), Arc::new(Credentials::default()));
    let accepted = fixture.grant();
    fixture
        .host
        .sessions()
        .lock_all(crate::session::LockReason::Background);
    assert_eq!(accepted.confirm().unwrap(), 1_900_000_000_123);
    assert_eq!(fixture.mark().unwrap(), Some(1_900_000_000_123));
    assert_eq!(format!("{accepted:?}"), "SecretAccessGrant([opaque])");
    drop(fixture);
    assert_eq!(accepted.confirm().unwrap_err(), ProfileError::Credentials);
}

// This adapter uses the existing authenticated local-state format, not test encryption.
struct EncryptedCredentials {
    directory: PathBuf,
    key: taypeer_storage::LocalStateKey,
}
impl EncryptedCredentials {
    fn path(&self, service: &str, account: &str) -> PathBuf {
        let address = format!("{}:{service}{account}", service.len());
        self.directory
            .join(Digest::of(address.as_bytes()).to_string())
    }
}
impl CredentialStore for EncryptedCredentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        let path = self.path(service, account);
        if !path.try_exists().map_err(|_| ProfileError::Io)? {
            return Ok(None);
        }
        taypeer_storage::LocalStateStore::read(&path, &self.key)
            .map(Some)
            .map_err(|_| ProfileError::Invalid)
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        let path = self.path(service, account);
        if path.try_exists().map_err(|_| ProfileError::Io)? {
            let mut store =
                taypeer_storage::LocalStateStore::open(&path).map_err(|_| ProfileError::Io)?;
            store.save(&self.key, bytes).map_err(|_| ProfileError::Io)
        } else {
            taypeer_storage::LocalStateStore::create(&path, &self.key, bytes)
                .map(drop)
                .map_err(|_| ProfileError::Io)
        }
    }
}
#[test]
fn access_metadata_reopens_through_real_encrypted_storage_without_document_or_secret_content() {
    let directory = tempfile::tempdir().unwrap();
    let encrypted = tempfile::tempdir().unwrap();
    let credentials = Arc::new(EncryptedCredentials {
        directory: encrypted.path().into(),
        key: taypeer_storage::LocalStateKey::from_secret(&[113; 32]),
    });
    let clock = Arc::new(Clock(AtomicI64::new(1_900_000_000_123)));
    let host = RuntimeHost::with_platform_credentials_and_access_clock(
        directory.path(),
        SessionController::new(Default::default()),
        credentials.clone(),
        clock.clone(),
    )
    .unwrap();
    let database = DatabaseId::new("PUBLIC encrypted db");
    let entry = EntryId::new("PUBLIC encrypted entry");
    host.secret_access_grant(database.clone(), entry.clone())
        .unwrap()
        .confirm()
        .unwrap();
    let clear = host
        .profile()
        .credentials
        .get(
            &host.profile().service(),
            &format!("state:{}", AccessRecord::key(&database, &entry)),
        )
        .unwrap()
        .unwrap();
    let value: serde_json::Value = serde_json::from_slice(&clear).unwrap();
    assert_eq!(
        value
            .as_object()
            .unwrap()
            .keys()
            .map(String::as_str)
            .collect::<std::collections::BTreeSet<_>>(),
        [
            "version",
            "database",
            "entry",
            "event",
            "owner",
            "accepted_order",
            "order",
            "utc_ms"
        ]
        .into_iter()
        .collect()
    );
    for file in std::fs::read_dir(encrypted.path()).unwrap() {
        let bytes = std::fs::read(file.unwrap().path()).unwrap();
        for plaintext in [
            b"PUBLIC encrypted db".as_slice(),
            b"PUBLIC encrypted entry".as_slice(),
            clear.as_slice(),
        ] {
            assert!(
                !bytes
                    .windows(plaintext.len())
                    .any(|window| window == plaintext)
            );
        }
    }
    drop(host);
    let reopened = RuntimeHost::with_platform_credentials_and_access_clock(
        directory.path(),
        SessionController::new(Default::default()),
        credentials,
        clock,
    )
    .unwrap();
    assert_eq!(
        reopened.accessed_at(&database, &entry).unwrap(),
        Some(1_900_000_000_123)
    );
    assert_eq!(std::fs::read_dir(directory.path()).unwrap().count(), 2); // public profile + host lease only
}
