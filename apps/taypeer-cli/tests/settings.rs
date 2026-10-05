//! Nonsecret settings exercise the real CLI without creating a native identity.
use std::{
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(profile: &Path, language: &str, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_taypeer-cli"))
        .arg("--profile")
        .arg(profile)
        .args(["--json", "--lang", language, "settings", "auto-lock"])
        .args(extra)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}
#[test]
fn auto_lock_settings_persist_without_native_credentials() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("PUBLIC settings");
    let output = run(&profile, "en", &[]);
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["idle_seconds"],
        300
    );
    assert!(!profile.exists());
    for language in ["en", "ru"] {
        let output = run(&profile, language, &["--seconds", "23"]);
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let output = run(&profile, language, &[]);
        assert_eq!(
            serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap()["idle_seconds"],
            23
        );
        assert!(!profile.join("profile.json").exists());
        let before = std::fs::read(profile.join("session-policy.json")).unwrap();
        assert!(
            !run(&profile, language, &["--seconds", "0"])
                .status
                .success()
        );
        assert_eq!(
            std::fs::read(profile.join("session-policy.json")).unwrap(),
            before
        );
    }
}

#[test]
fn startup_catalog_requires_no_identity_and_never_replaces_invalid_data() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("PUBLIC catalog");
    let list = || {
        Command::new(env!("CARGO_BIN_EXE_taypeer-cli"))
            .arg("--profile")
            .arg(&profile)
            .args(["--json", "db", "list"])
            .stdin(Stdio::null())
            .output()
            .unwrap()
    };
    let output = list();
    assert!(output.status.success());
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(&output.stdout).unwrap(),
        serde_json::json!([])
    );
    assert!(!profile.exists());
    std::fs::create_dir(&profile).unwrap();
    let catalog = profile.join("working-copies.json");
    std::fs::write(&catalog, b"PUBLIC invalid catalog").unwrap();
    let output = list();
    assert!(!output.status.success());
    assert!(output.stdout.is_empty());
    assert_eq!(std::fs::read(catalog).unwrap(), b"PUBLIC invalid catalog");
    assert!(!profile.join("profile.json").exists());
    assert!(!profile.join("public-credentials").exists());
}
