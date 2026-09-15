//! Synthetic local replicas feed real CLI processes; no network admission is implied.
#![cfg(target_os = "macos")]
mod support;
use serde_json::Value;
use std::{
    fs,
    io::Write,
    path::Path,
    process::{Command, Stdio},
};
use taypeer_core::OperationId;
use taypeer_document::{LifecycleAction, ObjectId};

const PASSWORD: &[u8] = b"PUBLIC lifecycle process master";
fn run(path: &Path, args: &[&str]) -> std::process::Output {
    let mut child = Command::new(env!("CARGO_BIN_EXE_taypeer-cli"))
        .arg("--profile")
        .arg(support::profile(path))
        .args(["--json", "--password-stdin", "--file"])
        .arg(path)
        .args(args)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(PASSWORD).unwrap();
    child.wait_with_output().unwrap()
}
fn ok(path: &Path, args: &[&str]) -> Value {
    let output = run(path, args);
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}

fn confirm(path: &Path, input: &Path, prepared: &Value, operation: &str) {
    fs::write(input, serde_json::to_vec(prepared).unwrap()).unwrap();
    let args = [
        "trash",
        "confirm",
        "--input",
        input.to_str().unwrap(),
        "--yes",
        "--operation",
        operation,
    ];
    let first = ok(path, &args);
    let bytes = fs::read(path).unwrap();
    assert_eq!(ok(path, &args), first);
    assert_eq!(fs::read(path).unwrap(), bytes);
}
#[test]
fn move_clone_trash_restore_and_purge_survive_new_processes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let input = directory.path().join("PUBLIC-selection.json");
    let (doc, a, b, entry) = support::fixture(&path);
    support::persist(&path, &doc, PASSWORD);
    ok(
        &path,
        &[
            "entry",
            "move",
            entry.as_str(),
            "--group",
            b.as_str(),
            "--operation",
            "PUBLIC move",
        ],
    );
    let cloned = ok(
        &path,
        &[
            "group",
            "clone",
            b.as_str(),
            "--parent",
            a.as_str(),
            "--operation",
            "PUBLIC clone",
        ],
    );
    assert_eq!(cloned.as_array().unwrap().len(), 2);
    let clone = cloned[0]["id"].as_str().unwrap();
    ok(
        &path,
        &[
            "group",
            "move",
            clone,
            "--before",
            a.as_str(),
            "--operation",
            "PUBLIC group move",
        ],
    );
    let prepared = ok(&path, &["group", "trash", clone]);
    assert_eq!(prepared["affected"].as_array().unwrap().len(), 2);
    confirm(&path, &input, &prepared, "PUBLIC trash");
    assert_eq!(ok(&path, &["trash", "list"]).as_array().unwrap().len(), 2);
    let prepared = ok(
        &path,
        &[
            "trash",
            "prepare",
            "restore",
            "group",
            clone,
            "--destination",
            a.as_str(),
        ],
    );
    confirm(&path, &input, &prepared, "PUBLIC restore");
    assert!(ok(&path, &["trash", "list"]).as_array().unwrap().is_empty());
    let prepared = ok(&path, &["group", "trash", clone]);
    confirm(&path, &input, &prepared, "PUBLIC trash again");
    let prepared = ok(&path, &["trash", "prepare", "purge", "group", clone]);
    confirm(&path, &input, &prepared, "PUBLIC purge");
    let tree = ok(&path, &["group", "tree"]);
    assert!(
        tree["groups"]
            .as_array()
            .unwrap()
            .iter()
            .all(|g| g["address"]["object"]["id"] != clone)
    );
    assert_eq!(ok(&path, &["entry", "list"]).as_array().unwrap().len(), 1);
    assert!(!tree.to_string().contains("PUBLIC_HIDDEN_PROCESS"));
}
#[test]
fn late_source_is_masked_and_recovery_receipt_survives_worker_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let (mut doc, _, b, entry) = support::fixture(&path);
    let mut late = doc.fork();
    let mut draft = late.begin_edit_entry(&entry).unwrap();
    draft.fields_mut().password = Some("PUBLIC_LATE_SECRET".into());
    late.save_entry(draft, 4).unwrap();
    for (action, id) in [
        (LifecycleAction::Trash, "PUBLIC trash"),
        (LifecycleAction::Purge, "PUBLIC purge"),
    ] {
        let prepared = doc
            .prepare_lifecycle(action, ObjectId::Entry(entry.clone()), None)
            .unwrap();
        doc.confirm_lifecycle(&prepared, &OperationId::new(id), 5)
            .unwrap();
    }
    doc.merge(&late).unwrap();
    support::persist(&path, &doc, PASSWORD);
    assert!(
        !run(&path, &["entry", "show", entry.as_str()])
            .status
            .success()
    );
    let sources = ok(&path, &["pending", "list"]);
    let id = sources[0]["id"].as_str().unwrap();
    let shown = ok(&path, &["pending", "show", id]);
    assert!(!shown.to_string().contains("PUBLIC_LATE_SECRET"));
    let args = [
        "pending",
        "restore",
        id,
        "--destination",
        b.as_str(),
        "--operation",
        "PUBLIC restore source",
    ];
    let restored = ok(&path, &args);
    assert_eq!(restored[0]["id"], entry.as_str());
    let bytes = fs::read(&path).unwrap();
    assert_eq!(ok(&path, &args), restored);
    assert_eq!(fs::read(&path).unwrap(), bytes);
    assert_eq!(
        ok(&path, &["entry", "reveal", entry.as_str()]),
        "PUBLIC_LATE_SECRET"
    );
    assert_eq!(
        ok(&path, &["history", "list", entry.as_str()])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    assert!(
        ok(&path, &["pending", "list"])
            .as_array()
            .unwrap()
            .is_empty()
    );
}
