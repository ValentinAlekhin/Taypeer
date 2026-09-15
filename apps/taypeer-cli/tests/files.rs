//! End-to-end encrypted file scenarios through real CLI and worker processes.

#![cfg(target_os = "macos")]
use serde_json::Value;
use std::{
    io::Write,
    path::Path,
    process::{Command, Output, Stdio},
};

fn run(path: Option<&Path>, args: &[&str], password: &[u8]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_taypeer-cli"));
    let database_path = path.unwrap_or_else(|| Path::new(args[2]));
    command
        .arg("--profile")
        .arg(database_path.parent().unwrap().join("profile"));
    command.args(["--json", "--password-stdin"]);
    if let Some(path) = path {
        command.arg("--file").arg(path);
    }
    let mut child = command
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(password).unwrap();
    child.wait_with_output().unwrap()
}

fn ok(output: Output) -> Value {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

#[test]
#[ignore = "native-keychain: explicitly opt in; may display macOS access dialogs"]
fn real_processes_preserve_files_history_and_addressed_updates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("public.taypeer");
    let password = b" PUBLIC master \n";
    let db = ok(run(
        None,
        &[
            "db",
            "create",
            path.to_str().unwrap(),
            "--name",
            "PUBLIC test",
        ],
        password,
    ));
    assert!(db["database"].is_string());
    let group = ok(run(
        Some(&path),
        &["group", "create", "--name", "PUBLIC group"],
        password,
    ));
    let group_id = group["id"].as_str().unwrap();
    let form = directory.path().join("public-input.json");
    std::fs::write(&form, br#"{"title":{"action":"set","value":"PUBLIC title"},"password":{"action":"set","value":"PUBLIC_SECRET_SENTINEL"},"username":{"action":"set","value":""}}"#).unwrap();
    let entry = ok(run(
        Some(&path),
        &[
            "entry",
            "create",
            "--group",
            group_id,
            "--input",
            form.to_str().unwrap(),
        ],
        password,
    ));
    let id = entry.as_str().unwrap();
    let shown = ok(run(Some(&path), &["entry", "show", id], password));
    assert_eq!(shown["username"], "");
    assert_eq!(shown["has_password"], true);
    assert!(!shown.to_string().contains("PUBLIC_SECRET_SENTINEL"));
    ok(run(
        Some(&path),
        &["entry", "update", id, "--title", "PUBLIC changed"],
        password,
    ));
    let revealed = ok(run(Some(&path), &["entry", "reveal", id], password));
    assert_eq!(revealed, "PUBLIC_SECRET_SENTINEL");
    let history = ok(run(Some(&path), &["history", "list", id], password));
    assert_eq!(history.as_array().unwrap().len(), 2);
    assert!(!history.to_string().contains("PUBLIC_SECRET_SENTINEL"));
    let search = ok(run(
        Some(&path),
        &["entry", "list", "--query", "PUBLIC_SECRET_SENTINEL"],
        password,
    ));
    assert_eq!(search, serde_json::json!([]));
    let disk = std::fs::read(&path).unwrap();
    assert!(
        !disk
            .windows(b"PUBLIC_SECRET_SENTINEL".len())
            .any(|w| w == b"PUBLIC_SECRET_SENTINEL")
    );
    let wrong = run(Some(&path), &["group", "list"], b"PUBLIC wrong");
    assert!(!wrong.status.success());
    assert!(wrong.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&wrong.stderr).contains("PUBLIC wrong"));
    assert_eq!(disk, std::fs::read(&path).unwrap());

    let clone_args = [
        "entry",
        "clone",
        id,
        "--group",
        group_id,
        "--operation",
        "PUBLIC durable clone",
    ];
    let cloned = ok(run(Some(&path), &clone_args, password));
    assert_ne!(cloned, entry);
    assert_eq!(ok(run(Some(&path), &clone_args, password)), cloned);
    assert_eq!(
        ok(run(Some(&path), &["entry", "list"], password))
            .as_array()
            .unwrap()
            .len(),
        2
    );

    let original = history[0]["id"].as_str().unwrap();
    let restore_args = [
        "history",
        "restore",
        id,
        original,
        "--group",
        group_id,
        "--operation",
        "PUBLIC durable restore",
    ];
    ok(run(Some(&path), &restore_args, password));
    ok(run(Some(&path), &restore_args, password));
    assert_eq!(
        ok(run(Some(&path), &["entry", "show", id], password))["title"],
        "PUBLIC title"
    );
    assert_eq!(
        ok(run(Some(&path), &["history", "list", id], password))
            .as_array()
            .unwrap()
            .len(),
        3
    );

    let purge_args = [
        "history",
        "purge",
        id,
        "--revision",
        original,
        "--yes",
        "--operation",
        "PUBLIC durable history purge",
    ];
    ok(run(Some(&path), &purge_args, password));
    ok(run(Some(&path), &purge_args, password));
    assert_eq!(
        ok(run(Some(&path), &["history", "list", id], password))
            .as_array()
            .unwrap()
            .len(),
        2
    );
}

#[test]
#[ignore = "native-keychain: explicitly opt in; may display macOS access dialogs"]
fn interrupted_draft_survives_worker_exit_and_requires_explicit_restore() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("public.taypeer");
    let password = b"PUBLIC master";
    ok(run(
        None,
        &["db", "create", path.to_str().unwrap(), "--name", "PUBLIC"],
        password,
    ));
    let group = ok(run(
        Some(&path),
        &["group", "create", "--name", "PUBLIC"],
        password,
    ));
    let created = ok(run(
        Some(&path),
        &[
            "entry",
            "create",
            "--group",
            group["id"].as_str().unwrap(),
            "--title",
            "PUBLIC title",
        ],
        password,
    ));
    let id = created.as_str().unwrap();
    let bad = run(
        Some(&path),
        &["entry", "update", id, "--title", ""],
        password,
    );
    assert!(!bad.status.success());
    let draft = ok(run(Some(&path), &["draft", "status"], password));
    assert_eq!(draft["active"], false);
    assert_eq!(draft["pending"]["entry_id"], id);
    assert!(
        !run(Some(&path), &["draft", "save"], password)
            .status
            .success()
    );
    ok(run(Some(&path), &["draft", "discard"], password));
    let shown = ok(run(Some(&path), &["entry", "show", id], password));
    assert_eq!(shown["title"], "PUBLIC title");
}
