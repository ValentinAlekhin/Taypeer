use ed25519_dalek::SigningKey;
use taypeer_encrypted_sync_spike::{Control, Fault, MAX_BYTES, container::Store};
use std::{fs, process::Command};
const PW: &[u8] = b"PUBLIC synthetic spike password";
fn setup() -> (tempfile::TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let manager = SigningKey::from_bytes(&[1; 32]);
    let id = manager.verifying_key().to_bytes();
    let store = Store::create(
        dir.path().join("db"),
        Control {
            group: [7; 16],
            seq: 0,
            epoch: 1,
            manager: id,
            members: [id].into(),
            previous: [0; 32],
        },
        PW,
        &manager,
    )
    .unwrap();
    (dir, store)
}
#[test]
fn rollback_queue_removal_and_modification_are_detected_by_local_anchor() {
    let (_dir, s) = setup();
    let old = fs::read(&s.path).unwrap();
    s.edit(
        PW,
        &SigningKey::from_bytes(&[1; 32]),
        &["field".into()],
        "PUBLIC",
        Fault::None,
    )
    .unwrap();
    let current = fs::read(&s.path).unwrap();
    fs::write(&s.path, &old).unwrap();
    assert!(s.load().is_err());
    fs::write(&s.path, &current).unwrap();
    let mut json: serde_json::Value = serde_json::from_slice(&current).unwrap();
    json["queue"] = serde_json::json!([]);
    fs::write(&s.path, serde_json::to_vec(&json).unwrap()).unwrap();
    assert!(s.load().is_err());
}
#[test]
fn exactly_ten_previous_containers_and_separate_pre_rotation_backup() {
    let (_dir, s) = setup();
    let manager = SigningKey::from_bytes(&[1; 32]);
    for n in 0..12 {
        s.edit(
            PW,
            &manager,
            &["field".into()],
            &format!("PUBLIC-{n}"),
            Fault::None,
        )
        .unwrap();
    }
    let root = s.load().unwrap().chain.root.hash();
    let snapshots: Vec<_> = fs::read_dir(s.path.with_extension("backups"))
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "snapshot"))
        .collect();
    assert_eq!(snapshots.len(), 10);
    for path in snapshots {
        taypeer_encrypted_sync_spike::container::Container::parse(&fs::read(path).unwrap(), root)
            .unwrap()
            .unlock(PW)
            .unwrap();
    }
}
#[test]
fn actual_child_exit_before_and_after_rename_recovers_whole_generation() {
    for fault in ["before", "after"] {
        let (dir, s) = setup();
        let root = s.load().unwrap().chain.root.hash();
        let root_path = dir.path().join("root");
        fs::write(&root_path, serde_json::to_vec(&root).unwrap()).unwrap();
        let status = Command::new(test_binary(
            "storage-probe",
            env!("CARGO_BIN_EXE_storage-probe"),
        ))
        .args([s.path.to_str().unwrap(), root_path.to_str().unwrap(), fault])
        .status()
        .unwrap();
        assert_eq!(status.code(), Some(77));
        let fresh = Store::at(s.path, root);
        fresh.load().unwrap().unlock(PW).unwrap();
        fresh
            .apply(PW, &SigningKey::from_bytes(&[1; 32]), Fault::None)
            .unwrap();
    }
}
#[test]
fn oversized_file_and_symlink_refused_before_overwrite() {
    use std::os::unix::fs::symlink;
    let (dir, s) = setup();
    let target = dir.path().join("target");
    fs::write(&target, b"unchanged").unwrap();
    let link = dir.path().join("link");
    symlink(&target, &link).unwrap();
    assert!(taypeer_encrypted_sync_spike::atomic_save(&link, b"replacement", Fault::None).is_err());
    assert_eq!(fs::read(target).unwrap(), b"unchanged");
    fs::File::create(&s.path)
        .unwrap()
        .set_len(MAX_BYTES as u64 + 1)
        .unwrap();
    assert!(s.load().is_err());
}
#[test]
fn calibrated_parameters_survive_save_and_unlock() {
    let (_dir, s) = setup();
    let manager = SigningKey::from_bytes(&[1; 32]);
    let result = s.calibrate_kdf(PW, &manager, 500).unwrap();
    assert!(result.within_tolerance);
    s.edit(
        PW,
        &manager,
        &["field".into()],
        "PUBLIC calibrated",
        Fault::None,
    )
    .unwrap();
    assert_eq!(
        s.load().unwrap().unlock(PW).unwrap().kdf.iterations,
        result.profile.iterations
    );
}

fn test_binary(name: &str, built: &str) -> std::path::PathBuf {
    std::env::var_os("TAYPEER_TEST_BIN_DIR")
        .map(|dir| std::path::PathBuf::from(dir).join(name))
        .unwrap_or_else(|| built.into())
}
