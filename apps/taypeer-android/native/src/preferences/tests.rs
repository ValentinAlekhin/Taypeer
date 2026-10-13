//! PUBLIC nonsecret preferences survive host restart and cannot bypass validation.
use super::*;
use crate::CredentialPort;
use std::{
    collections::BTreeMap,
    sync::{Arc, Barrier, Mutex},
};

#[derive(Clone, Default)]
struct Credentials(Arc<Mutex<CredentialValues>>);
type CredentialValues = BTreeMap<(String, String), Vec<u8>>;
impl CredentialPort for Credentials {
    fn get(&self, service: String, account: String) -> Result<Option<Vec<u8>>, AndroidError> {
        Ok(self.0.lock().unwrap().get(&(service, account)).cloned())
    }
    fn set(&self, service: String, account: String, bytes: Vec<u8>) -> Result<(), AndroidError> {
        self.0.lock().unwrap().insert((service, account), bytes);
        Ok(())
    }
}
fn host(profile: &Path, credentials: Credentials) -> Arc<Host> {
    Host::new(profile.to_str().unwrap().to_owned(), Box::new(credentials)).unwrap()
}

#[test]
fn policies_load_on_host_creation_and_survive_restart_without_ui_state() {
    let profile = tempfile::tempdir().unwrap();
    let credentials = Credentials::default();
    let owner = host(profile.path(), credentials.clone());
    assert_eq!(
        owner.device_settings().unwrap(),
        DeviceSettingsView {
            idle_seconds: 300,
            clipboard_seconds: Some(30),
            device_name: "Taypeer".into(),
        }
    );
    assert_eq!(owner.set_idle_seconds(17).unwrap().idle_seconds, 17);
    assert_eq!(
        owner.set_clipboard_seconds(None).unwrap().clipboard_seconds,
        None
    );
    drop(owner);
    let restored = host(profile.path(), credentials.clone());
    assert_eq!(
        restored.device_settings().unwrap(),
        DeviceSettingsView {
            idle_seconds: 17,
            clipboard_seconds: None,
            device_name: "Taypeer".into(),
        }
    );
    restored.set_clipboard_seconds(Some(u32::MAX)).unwrap();
    drop(restored);
    assert_eq!(
        host(profile.path(), credentials)
            .device_settings()
            .unwrap()
            .clipboard_seconds,
        Some(u32::MAX)
    );
}

#[test]
fn invalid_values_and_blocked_saves_do_not_hot_apply_a_policy() {
    let profile = tempfile::tempdir().unwrap();
    let owner = host(profile.path(), Credentials::default());
    let before = owner.device_settings().unwrap();
    assert!(matches!(
        owner.set_idle_seconds(0),
        Err(AndroidError::InvalidOptions)
    ));
    assert!(matches!(
        owner.set_clipboard_seconds(Some(0)),
        Err(AndroidError::InvalidOptions)
    ));
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(profile.path().join("session-policy.lock"))
        .unwrap();
    taypeer_storage::try_lock_exclusive(&lock).unwrap();
    assert!(matches!(
        owner.set_idle_seconds(17),
        Err(AndroidError::ProfileBusy)
    ));
    assert_eq!(owner.device_settings().unwrap(), before);
    drop(lock);
    let lock = OpenOptions::new()
        .create(true)
        .truncate(false)
        .read(true)
        .write(true)
        .open(profile.path().join(CLIPBOARD_LOCK))
        .unwrap();
    taypeer_storage::try_lock_exclusive(&lock).unwrap();
    assert!(matches!(
        owner.set_clipboard_seconds(None),
        Err(AndroidError::ProfileBusy)
    ));
    assert_eq!(owner.device_settings().unwrap(), before);
}

#[test]
fn malformed_preferences_fail_startup_and_are_preserved_on_set() {
    for malformed in [
        r#"{"version":1,"clipboard":{"mode":"timed","seconds":0}}"#,
        r#"{"version":99,"clipboard":{"mode":"never"}}"#,
        r#"{"version":1}"#,
        r#"{"version":1,"clipboard":{"mode":"never","future":17}}"#,
    ] {
        let profile = tempfile::tempdir().unwrap();
        let credentials = Credentials::default();
        let owner = host(profile.path(), credentials.clone());
        let path = profile.path().join(CLIPBOARD_FILE);
        std::fs::write(&path, malformed).unwrap();
        assert!(matches!(
            owner.set_clipboard_seconds(Some(17)),
            Err(AndroidError::ProfileInvalid)
        ));
        assert_eq!(owner.device_settings().unwrap().clipboard_seconds, Some(30));
        drop(owner);
        assert!(matches!(
            Host::new(
                profile.path().to_str().unwrap().into(),
                Box::new(credentials)
            ),
            Err(AndroidError::ProfileInvalid)
        ));
        assert_eq!(std::fs::read_to_string(path).unwrap(), malformed);
    }
    let profile = tempfile::tempdir().unwrap();
    std::fs::write(
        profile.path().join("session-policy.json"),
        r#"{"version":1,"policy":{"idle_seconds":0}}"#,
    )
    .unwrap();
    assert!(matches!(
        Host::new(
            profile.path().to_str().unwrap().into(),
            Box::new(Credentials::default())
        ),
        Err(AndroidError::ProfileInvalid)
    ));
}

#[test]
fn concurrent_setters_serialize_publication_and_hot_application() {
    let profile = tempfile::tempdir().unwrap();
    let owner = host(profile.path(), Credentials::default());
    let barrier = Arc::new(Barrier::new(3));
    std::thread::scope(|scope| {
        for seconds in [17, 29] {
            let owner = Arc::clone(&owner);
            let barrier = Arc::clone(&barrier);
            scope.spawn(move || {
                barrier.wait();
                owner.set_idle_seconds(seconds).unwrap();
            });
        }
        barrier.wait();
    });
    assert_eq!(
        SessionSettings::load(profile.path())
            .unwrap()
            .idle_seconds(),
        owner.device_settings().unwrap().idle_seconds
    );
}

#[test]
fn local_name_returns_typed_validation_errors_and_survives_native_host_restart() {
    let profile = tempfile::tempdir().unwrap();
    let credentials = Credentials::default();
    let owner = host(profile.path(), credentials.clone());
    for value in ["", " \t\n ", "\u{2003}"] {
        assert!(matches!(
            owner.set_device_name(value.into()),
            Err(AndroidError::DeviceNameRequired)
        ));
    }
    assert!(matches!(
        owner.set_device_name("П".repeat(129)),
        Err(AndroidError::DeviceNameTooLong)
    ));
    assert_eq!(owner.device_settings().unwrap().device_name, "Taypeer");
    let name = " PUBLIC Android / Андроид ";
    let confirmed = owner.set_device_name(name.into()).unwrap();
    assert_eq!(confirmed.device_name, name);
    assert_eq!(confirmed.idle_seconds, 300);
    assert_eq!(confirmed.clipboard_seconds, Some(30));
    assert!(!format!("{confirmed:?}").contains(name));
    drop(owner);
    let restored = host(profile.path(), credentials);
    assert_eq!(restored.device_settings().unwrap().device_name, name);
    restored.set_device_name("🧪".repeat(64)).unwrap();
    assert!(matches!(
        restored.set_device_name("🧪".repeat(65)),
        Err(AndroidError::DeviceNameTooLong)
    ));
    assert_eq!(
        restored.device_settings().unwrap().device_name,
        "🧪".repeat(64)
    );
}

#[test]
fn failed_name_publication_preserves_the_applied_view_and_existing_configuration() {
    let profile = tempfile::tempdir().unwrap();
    let owner = host(profile.path(), Credentials::default());
    owner.set_device_name("PUBLIC confirmed".into()).unwrap();
    let before = owner.device_settings().unwrap();
    let path = profile.path().join("device-name.json");
    let bytes = std::fs::read(&path).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(profile.path().join("device-name.lock"))
        .unwrap();
    taypeer_storage::try_lock_exclusive(&lock).unwrap();
    assert!(matches!(
        owner.set_device_name("PUBLIC blocked".into()),
        Err(AndroidError::ProfileBusy)
    ));
    assert_eq!(owner.device_settings().unwrap(), before);
    assert_eq!(std::fs::read(&path).unwrap(), bytes);
    drop(lock);
    let malformed = br#"{"version":99,"name":"PUBLIC future"}"#;
    std::fs::write(&path, malformed).unwrap();
    assert!(matches!(
        owner.set_device_name("PUBLIC replacement".into()),
        Err(AndroidError::ProfileInvalid)
    ));
    assert_eq!(owner.device_settings().unwrap(), before);
    assert_eq!(std::fs::read(&path).unwrap(), malformed);
    let credentials = Credentials::default();
    drop(owner);
    assert!(matches!(
        Host::new(
            profile.path().to_str().unwrap().into(),
            Box::new(credentials)
        ),
        Err(AndroidError::ProfileInvalid)
    ));
    assert_eq!(std::fs::read(path).unwrap(), malformed);
}

#[test]
fn an_uncertain_name_reply_retains_applied_values_until_an_explicit_confirmed_retry() {
    for retry_prior_name in [false, true] {
        let profile = tempfile::tempdir().unwrap();
        let owner = host(profile.path(), Credentials::default());
        let before = owner.device_settings().unwrap();
        let name =
            LocalDeviceName::try_from("PUBLIC published without confirmation".to_owned()).unwrap();
        {
            let mut preferences = owner.preferences.lock().unwrap();
            let result = preferences.confirm_name(name.clone(), |candidate| {
                DeviceNameSettings::save(profile.path(), candidate).map_err(host_error)?;
                // The real file changed, but its caller lost durability confirmation.
                // Storage's publication tests separately inject directory-sync failure.
                Err(host_error(ProfileError::CommitUncertain.into()))
            });
            assert!(matches!(result, Err(AndroidError::CommitUncertain)));
        }
        assert_eq!(owner.device_settings().unwrap(), before);
        assert_eq!(DeviceNameSettings::load(profile.path()).unwrap(), name);
        let retry = if retry_prior_name {
            before.device_name.clone()
        } else {
            name.into_string()
        };
        let confirmed = owner.set_device_name(retry.clone()).unwrap();
        assert_eq!(confirmed.device_name, retry);
        assert_eq!(
            DeviceNameSettings::load(profile.path()).unwrap().as_str(),
            retry
        );
    }
}

#[test]
fn concurrent_name_and_policy_updates_remain_coherent_and_names_do_not_cross_profiles() {
    let profile = tempfile::tempdir().unwrap();
    let other = tempfile::tempdir().unwrap();
    let credentials = Credentials::default();
    let owner = host(profile.path(), credentials.clone());
    let barrier = Arc::new(Barrier::new(4));
    std::thread::scope(|scope| {
        let name_owner = Arc::clone(&owner);
        let name_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            name_barrier.wait();
            name_owner
                .set_device_name("PUBLIC parallel name".into())
                .unwrap();
        });
        let idle_owner = Arc::clone(&owner);
        let idle_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            idle_barrier.wait();
            idle_owner.set_idle_seconds(17).unwrap();
        });
        let clipboard_owner = Arc::clone(&owner);
        let clipboard_barrier = Arc::clone(&barrier);
        scope.spawn(move || {
            clipboard_barrier.wait();
            clipboard_owner.set_clipboard_seconds(None).unwrap();
        });
        barrier.wait();
    });
    let confirmed = owner.device_settings().unwrap();
    assert_eq!(confirmed.device_name, "PUBLIC parallel name");
    assert_eq!(confirmed.idle_seconds, 17);
    assert_eq!(confirmed.clipboard_seconds, None);
    assert_eq!(
        DeviceNameSettings::load(profile.path()).unwrap().as_str(),
        confirmed.device_name
    );
    drop(owner);
    assert_eq!(
        host(profile.path(), credentials.clone())
            .device_settings()
            .unwrap(),
        confirmed
    );
    assert_eq!(
        host(other.path(), credentials)
            .device_settings()
            .unwrap()
            .device_name,
        "Taypeer"
    );
}
