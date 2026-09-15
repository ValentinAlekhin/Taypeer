//! PUBLIC synthetic file inspection; deliberately requires neither password input nor Keychain.
use std::{
    fs,
    process::{Command, Stdio},
};
use taypeer_core::DatabasePolicy;
use taypeer_services::DatabaseService;
use taypeer_trust::{AuthorKey, Identity, TransportKey};

#[test]
fn compatibility_inspection_uses_no_profile_or_password_and_preserves_the_file() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC compatibility.taypeer");
    let profile = directory.path().join("must-not-create-profile");
    let author = AuthorKey::from_seed(&[85; 32]);
    let transport = TransportKey::from_seed(&[86; 32]);
    let identity = Identity::new(author.public(), transport.public()).unwrap();
    let seed = DatabaseService::prepare_managed(
        "PUBLIC concealed name".into(),
        b"PUBLIC concealed password",
        &author,
        identity,
        1,
        DatabasePolicy::new(1024, 2048, 500).unwrap(),
    )
    .unwrap();
    drop(seed.create(&path, &transport, None).unwrap());
    let before = fs::read(&path).unwrap();
    for lang in ["en", "ru"] {
        let output = Command::new(env!("CARGO_BIN_EXE_taypeer-cli"))
            .arg("--profile")
            .arg(&profile)
            .args(["--json", "--lang", lang, "--file"])
            .arg(&path)
            .args(["db", "compatibility"])
            .stdin(Stdio::null())
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: serde_json::Value = serde_json::from_slice(&output.stdout).unwrap();
        assert_eq!(value["format"]["schema"]["schema_version"], 5);
        for access in ["read", "write", "receive"] {
            assert_eq!(value["format"][access]["status"], "supported");
        }
        assert_eq!(value["locked"], true);
        assert!(value["admitted"].is_null());
        assert!(!String::from_utf8_lossy(&output.stdout).contains("concealed"));
        assert!(!profile.exists());
        assert_eq!(fs::read(&path).unwrap(), before);
    }
    let mut old = before;
    old[10] = 4;
    fs::write(&path, &old).unwrap();
    let output = Command::new(env!("CARGO_BIN_EXE_taypeer-cli"))
        .args(["--json", "--lang", "ru", "--file"])
        .arg(&path)
        .args(["db", "compatibility"])
        .stdin(Stdio::null())
        .output()
        .unwrap();
    assert!(!output.status.success());
    let error: serde_json::Value = serde_json::from_slice(&output.stderr).unwrap();
    assert_eq!(error["error"]["code"], "compatibility_encoding");
    assert_eq!(fs::read(path).unwrap(), old);
}
