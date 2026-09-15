use super::*;
use std::sync::atomic::{AtomicU64, Ordering};
struct FakeClock(AtomicU64);
impl Clock for FakeClock {
    fn now(&self) -> Duration {
        Duration::from_millis(self.0.load(Ordering::SeqCst))
    }
}
#[test]
fn foreground_activity_keeps_all_databases_alive_but_background_reads_do_not() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let sessions = SessionController::with_clock(SessionPolicy::new(2).unwrap(), clock.clone());
    let directory = tempfile::tempdir().unwrap();
    let first = crate::process::tests::open(&sessions, "normal", &directory.path().join("first"));
    let second = crate::process::tests::open(&sessions, "normal", &directory.path().join("second"));
    clock.0.store(1_900, Ordering::SeqCst);
    sessions.activity().touch(); // Input in either database renews the shared timer.
    clock.0.store(3_800, Ordering::SeqCst);
    first.request(&crate::Command::Groups, false).unwrap();
    second.request(&crate::Command::Groups, false).unwrap();
    clock.0.store(3_900, Ordering::SeqCst);
    sessions.activity().touch(); // An overdue touch must close both old generations first.
    for client in [first, second] {
        assert_eq!(
            client.control.check(false),
            Err(RuntimeError::SessionClosed(LockReason::Idle))
        );
        assert_eq!(
            client.control.wait_closed().unwrap().draft,
            DraftDisposition::Preserved
        );
    }
}
#[test]
fn input_deadline_and_policy_changes_use_monotonic_time_without_background_renewal() {
    let clock = Arc::new(FakeClock(AtomicU64::new(0)));
    let sessions = SessionController::with_clock(SessionPolicy::default(), clock.clone());
    let input = sessions.activity();
    clock.0.store(299_999, Ordering::SeqCst);
    assert_eq!(input.epoch(), 0);
    input.touch();
    clock.0.store(599_998, Ordering::SeqCst);
    assert_eq!(input.epoch(), 0);
    clock.0.store(599_999, Ordering::SeqCst);
    input.touch();
    assert_eq!(input.epoch(), 1); // Touch at the boundary expires first.
    assert_eq!(input.reason(), Some(LockReason::Idle));
    sessions.set_policy(SessionPolicy::new(2).unwrap());
    clock.0.store(601_999, Ordering::SeqCst);
    sessions.statuses(); // Reading status cannot extend access.
    assert_eq!(input.epoch(), 2);
    sessions.lock_all(LockReason::Background);
    assert_eq!(input.reason(), Some(LockReason::Background));
}
#[test]
fn local_settings_are_durable_and_reject_invalid_or_unknown_values() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC profile");
    assert_eq!(
        SessionSettings::load(&path).unwrap(),
        SessionPolicy::default()
    );
    assert!(!path.exists());
    let policy = SessionPolicy::new(17).unwrap();
    SessionSettings::save(&path, policy).unwrap();
    assert_eq!(SessionSettings::load(&path).unwrap(), policy);
    assert!(!path.join("profile.json").exists());
    for bytes in [
        r#"{"version":1,"policy":{"idle_seconds":0}}"#,
        r#"{"version":99,"policy":{"idle_seconds":17}}"#,
    ] {
        std::fs::write(path.join("session-policy.json"), bytes).unwrap();
        assert!(SessionSettings::load(&path).is_err());
        assert_eq!(
            std::fs::read_to_string(path.join("session-policy.json")).unwrap(),
            bytes
        );
    }
}
