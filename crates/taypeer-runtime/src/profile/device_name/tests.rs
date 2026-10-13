//! PUBLIC local labels exercise validation, durable configuration and refusal preservation.
use super::*;

fn name(value: &str) -> LocalDeviceName {
    value.to_owned().try_into().unwrap()
}

#[test]
fn validation_preserves_original_text_and_counts_utf8_bytes() {
    assert_eq!(LocalDeviceName::default().as_str(), "Taypeer");
    let original = "  PUBLIC Android / Андроид  ";
    assert_eq!(name(original).as_str(), original);
    for value in ["", " \t\r\n", "\u{2003}"] {
        assert_eq!(
            LocalDeviceName::validate(value),
            Err(DeviceNameError::Required)
        );
    }
    for (at_limit, over_limit) in [
        ("P".repeat(256), "P".repeat(257)),
        ("П".repeat(128), "П".repeat(129)),
        ("🧪".repeat(64), "🧪".repeat(65)),
    ] {
        assert_eq!(name(&at_limit).as_str(), at_limit);
        assert_eq!(
            LocalDeviceName::validate(&over_limit),
            Err(DeviceNameError::TooLong)
        );
    }
    assert!(!format!("{:?}", name(original)).contains(original));
}

#[test]
fn deserialization_cannot_construct_an_invalid_local_name() {
    for value in [String::new(), " \t ".into(), "P".repeat(257)] {
        let encoded = serde_json::to_string(&value).unwrap();
        assert!(serde_json::from_str::<LocalDeviceName>(&encoded).is_err());
    }
    let original = " PUBLIC exact label ";
    let encoded = serde_json::to_string(&name(original)).unwrap();
    assert_eq!(
        serde_json::from_str::<LocalDeviceName>(&encoded)
            .unwrap()
            .as_str(),
        original
    );
}

#[test]
fn saved_name_survives_independent_loads_and_stays_in_its_local_profile() {
    let first = tempfile::tempdir().unwrap();
    let second = tempfile::tempdir().unwrap();
    assert_eq!(
        DeviceNameSettings::load(first.path()).unwrap().as_str(),
        "Taypeer"
    );
    assert!(
        !first.path().join(SETTINGS_FILE).exists(),
        "reading a default must not create a configuration file"
    );
    let chosen = name(" PUBLIC device / устройство ");
    DeviceNameSettings::save(first.path(), &chosen).unwrap();
    assert_eq!(DeviceNameSettings::load(first.path()).unwrap(), chosen);
    assert_eq!(
        DeviceNameSettings::load(second.path()).unwrap().as_str(),
        "Taypeer"
    );
    assert_eq!(
        std::fs::read_to_string(first.path().join(SETTINGS_FILE)).unwrap(),
        r#"{"version":1,"name":" PUBLIC device / устройство "}"#
    );
}

#[test]
fn malformed_future_and_oversized_settings_are_never_replaced_or_defaulted() {
    for malformed in [
        b"PUBLIC malformed configuration".to_vec(),
        br#"{"version":99,"name":"PUBLIC future"}"#.to_vec(),
        br#"{"version":1,"name":"PUBLIC future","unknown":true}"#.to_vec(),
        br#"{"version":1,"name":"  "}"#.to_vec(),
        br#"{"version":1}"#.to_vec(),
        serde_json::to_vec(&serde_json::json!({"version":1,"name":"P".repeat(257)})).unwrap(),
        vec![b'P'; 4097],
    ] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(SETTINGS_FILE);
        std::fs::write(&path, &malformed).unwrap();
        assert!(matches!(
            DeviceNameSettings::load(directory.path()),
            Err(RuntimeError::Profile(ProfileError::Invalid))
        ));
        assert!(matches!(
            DeviceNameSettings::save(directory.path(), &name("PUBLIC correction")),
            Err(RuntimeError::Profile(ProfileError::Invalid))
        ));
        assert_eq!(std::fs::read(path).unwrap(), malformed);
    }
}

#[test]
fn a_locked_name_write_refuses_before_replacing_the_confirmed_label() {
    let directory = tempfile::tempdir().unwrap();
    DeviceNameSettings::save(directory.path(), &name("PUBLIC confirmed")).unwrap();
    let before = std::fs::read(directory.path().join(SETTINGS_FILE)).unwrap();
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .open(directory.path().join(SETTINGS_LOCK))
        .unwrap();
    taypeer_storage::try_lock_exclusive(&lock).unwrap();
    assert!(matches!(
        DeviceNameSettings::save(directory.path(), &name("PUBLIC unconfirmed")),
        Err(RuntimeError::Profile(ProfileError::Busy))
    ));
    assert_eq!(
        std::fs::read(directory.path().join(SETTINGS_FILE)).unwrap(),
        before
    );
    drop(lock);
    DeviceNameSettings::save(directory.path(), &name("PUBLIC retry")).unwrap();
    assert_eq!(
        DeviceNameSettings::load(directory.path()).unwrap().as_str(),
        "PUBLIC retry"
    );
}

#[test]
fn an_unreadable_destination_is_not_treated_as_missing_configuration() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join(SETTINGS_FILE);
    std::fs::create_dir(&path).unwrap();
    std::fs::write(path.join("PUBLIC sentinel"), b"PUBLIC preserve").unwrap();
    assert!(matches!(
        DeviceNameSettings::load(directory.path()),
        Err(RuntimeError::Profile(ProfileError::Io))
    ));
    assert!(matches!(
        DeviceNameSettings::save(directory.path(), &name("PUBLIC candidate")),
        Err(RuntimeError::Profile(ProfileError::Io))
    ));
    assert_eq!(
        std::fs::read(path.join("PUBLIC sentinel")).unwrap(),
        b"PUBLIC preserve"
    );
}
