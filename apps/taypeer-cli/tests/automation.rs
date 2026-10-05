//! Public fixture credentials exercise real CLI and worker processes without native prompts.
#![cfg(feature = "ui-test-support")]

use serde_json::{Value, json};
use std::{
    io::Write,
    path::{Path, PathBuf},
    process::{Command, Output, Stdio},
};

const PASSWORD: &[u8] = b" PUBLIC automation master \n";

struct Client {
    profile: PathBuf,
    file: Option<PathBuf>,
}
impl Client {
    fn new(directory: &Path) -> Self {
        Self {
            profile: directory.join("profile"),
            file: None,
        }
    }
    fn run(&self, args: &[&str], password: Option<&[u8]>) -> Output {
        let mut command = Command::new(env!("CARGO_BIN_EXE_taypeer-cli"));
        command
            .arg("--profile")
            .arg(&self.profile)
            .args(["--public-fixture-profile", "--json"]);
        if let Some(file) = &self.file {
            command.arg("--file").arg(file);
        }
        if password.is_some() {
            command.arg("--password-stdin");
        }
        let mut child = command
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        if let Some(password) = password {
            child.stdin.take().unwrap().write_all(password).unwrap();
        }
        child.wait_with_output().unwrap()
    }
    fn ok(&self, args: &[&str]) -> Value {
        checked(self.run(args, self.file.as_ref().map(|_| PASSWORD)))
    }
    fn create(&mut self) -> Value {
        let result = checked(self.run(
            &[
                "db",
                "create",
                "--name",
                "PUBLIC automation",
                "--operation",
                "PUBLIC creation",
            ],
            Some(PASSWORD),
        ));
        self.file = Some(PathBuf::from(result["file"].as_str().unwrap()));
        let profile = taypeer_runtime::profile::NativeProfile::load_test(&self.profile).unwrap();
        let identity = profile
            .identity()
            .unwrap()
            .expect("Worker enrolled the fixture identity");
        let archive =
            taypeer_storage::ArchiveSnapshot::open(self.file.as_ref().unwrap(), None).unwrap();
        assert_eq!(archive.chain().head().manager, identity.device);
        result
    }
    fn write_input(&self, name: &str, value: Value) -> PathBuf {
        let path = self.profile.parent().unwrap().join(name);
        std::fs::write(&path, serde_json::to_vec(&value).unwrap()).unwrap();
        path
    }
}
fn checked(output: Output) -> Value {
    assert!(
        output.status.success(),
        "CLI failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    serde_json::from_slice(&output.stdout).unwrap()
}
fn identity(view: &Value) -> (&str, u64) {
    (
        view["identity"]["draft"].as_str().unwrap(),
        view["identity"]["revision"].as_u64().unwrap(),
    )
}
fn snapshot(client: &Client, view: &Value, operation: &str) -> Value {
    let (id, revision) = identity(view);
    client.ok(&[
        "draft",
        "snapshot",
        id,
        "--revision",
        &revision.to_string(),
        "--operation",
        operation,
    ])
}

#[test]
fn catalog_creation_ungrouped_and_external_copy_preserve_sources() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::new(directory.path());
    assert_eq!(client.ok(&["db", "list"]), json!([]));
    assert!(!client.profile.exists());
    let created = client.create();
    let file = client.file.take().unwrap();
    assert!(
        file.starts_with(
            client
                .profile
                .canonicalize()
                .unwrap()
                .join("working-copies")
        )
    );
    let repeat = checked(client.run(
        &[
            "db",
            "create",
            "--name",
            "PUBLIC automation",
            "--operation",
            "PUBLIC creation",
        ],
        Some(PASSWORD),
    ));
    assert_eq!(repeat, created);
    let catalog = client.ok(&["db", "list"]);
    assert_eq!(catalog.as_array().unwrap().len(), 1);
    assert_eq!(catalog[0]["locked"], true);
    assert_eq!(
        client.ok(&["db", "use", created["database"].as_str().unwrap()]),
        Value::Null
    );
    assert!(!catalog.to_string().contains("PUBLIC automation"));
    client.file = Some(file.clone());
    let input = client.write_input("PUBLIC-entry.json", json!({"title":{"action":"set","value":"PUBLIC ungrouped"},"password":{"action":"set","value":"PUBLIC_SECRET_SENTINEL"}}));
    let entry = client.ok(&[
        "entry",
        "create",
        "--input",
        input.to_str().unwrap(),
        "--operation",
        "PUBLIC entry",
    ]);
    let id = entry.as_str().unwrap();
    assert_eq!(client.ok(&["group", "list"]), json!([]));
    let shown = client.ok(&["entry", "show", id]);
    assert_eq!(shown["has_password"], true);
    assert!(!shown.to_string().contains("PUBLIC_SECRET_SENTINEL"));
    let history = client.ok(&["history", "list", id]);
    assert!(!history.to_string().contains("PUBLIC_SECRET_SENTINEL"));
    let revision = history[0]["id"].as_str().unwrap();
    assert_eq!(
        client.ok(&["history", "reveal", id, revision]),
        "PUBLIC_SECRET_SENTINEL"
    );
    assert!(
        !client
            .ok(&["history", "alternatives", id])
            .to_string()
            .contains("PUBLIC_SECRET_SENTINEL")
    );
    let cloned = client.ok(&["entry", "clone", id, "--operation", "PUBLIC clone"]);
    assert_eq!(
        client.ok(&["entry", "clone", id, "--operation", "PUBLIC clone"]),
        cloned
    );
    assert_eq!(client.ok(&["entry", "list"]).as_array().unwrap().len(), 2);
    let source = std::fs::read(&file).unwrap();
    let external_directory = tempfile::tempdir().unwrap();
    let external = Client::new(external_directory.path());
    let opened = checked(external.run(&["db", "open", file.to_str().unwrap()], Some(PASSWORD)));
    let internal = PathBuf::from(opened["file"].as_str().unwrap());
    assert_ne!(internal, file);
    assert!(
        internal.starts_with(
            external
                .profile
                .canonicalize()
                .unwrap()
                .join("working-copies")
        )
    );
    assert_eq!(std::fs::read(&file).unwrap(), source);
    let readonly = Client {
        file: Some(internal),
        ..external
    };
    assert_eq!(readonly.ok(&["entry", "list"]).as_array().unwrap().len(), 2);
    let denied = readonly.run(
        &["entry", "update", id, "--title", "PUBLIC denied"],
        Some(PASSWORD),
    );
    assert!(!denied.status.success());
    assert_eq!(std::fs::read(&file).unwrap(), source);
    let wrong = client.run(&["entry", "list"], Some(b"PUBLIC wrong"));
    assert!(!wrong.status.success());
    assert!(wrong.stdout.is_empty());
    assert!(!String::from_utf8_lossy(&wrong.stderr).contains("PUBLIC wrong"));
}

#[test]
fn independent_local_forms_continue_and_snapshot_retries_preserve_newer_input() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::new(directory.path());
    client.create();
    let a = client.ok(&["entry", "create", "--title", "PUBLIC A"]);
    let b = client.ok(&["entry", "create", "--title", "PUBLIC B"]);
    let a = a.as_str().unwrap();
    let b = b.as_str().unwrap();
    let first = client.ok(&["draft", "edit", a, "--title", ""]);
    let aid = identity(&first).0;
    let unfinished = client.ok(&["draft", "update", "--draft", aid, "--title", ""]);
    assert!(
        snapshot(&client, &unfinished, "PUBLIC incomplete")
            .get("LocalDraftSaved")
            .is_some()
    );
    assert_eq!(client.ok(&["entry", "show", a])["title"], "PUBLIC A");
    let second = client.ok(&["draft", "edit", b, "--title", ""]);
    let bid = identity(&second).0;
    client.ok(&["draft", "update", "--draft", bid, "--title", ""]);
    let list = client.ok(&["draft", "list"]);
    assert_eq!(list.as_array().unwrap().len(), 2);
    assert!(!list.to_string().contains("PUBLIC A"));
    let continued = client.ok(&["draft", "edit", a]);
    assert_eq!(identity(&continued).0, aid);
    assert_eq!(continued["fields"]["title"], "");
    let old = client.ok(&["draft", "update", "--draft", aid, "--title", "PUBLIC saved"]);
    let receipt = snapshot(&client, &old, "PUBLIC exact snapshot");
    assert!(receipt.get("Saved").is_some());
    let fresh = client.ok(&["draft", "edit", a, "--title", "PUBLIC newer local"]);
    assert_eq!(snapshot(&client, &old, "PUBLIC exact snapshot"), receipt);
    let current = client.ok(&["draft", "resume", identity(&fresh).0]);
    assert_eq!(identity(&current), identity(&fresh));
    assert_eq!(current["fields"]["title"], "PUBLIC newer local");
    assert_eq!(client.ok(&["entry", "show", a])["title"], "PUBLIC saved");
    assert_eq!(
        client.ok(&["history", "list", a]).as_array().unwrap().len(),
        2
    );
    client.ok(&["draft", "delete", bid]);
    assert_eq!(client.ok(&["draft", "list"]).as_array().unwrap().len(), 1);
    assert_eq!(client.ok(&["entry", "show", b])["title"], "PUBLIC B");
}

#[test]
fn metadata_history_purge_requires_confirmation_and_relocation_preserves_drafts() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::new(directory.path());
    client.create();
    let input = client.write_input("PUBLIC-group.json", json!({"name":{"action":"set","value":"PUBLIC group"},"description":{"action":"set","value":"PUBLIC description"}}));
    let group = client.ok(&[
        "draft",
        "group",
        "create",
        "--input",
        input.to_str().unwrap(),
    ]);
    let groupid = group["identity"]["target"]["NewGroup"]["group"]
        .as_str()
        .unwrap_or_else(|| panic!("PUBLIC group view: {group}"));
    snapshot(&client, &group, "PUBLIC group snapshot");
    let input = client.write_input(
        "PUBLIC-db.json",
        json!({"description":{"action":"set","value":"PUBLIC database description"}}),
    );
    let db = client.ok(&[
        "draft",
        "database",
        "edit",
        "--input",
        input.to_str().unwrap(),
    ]);
    snapshot(&client, &db, "PUBLIC database snapshot");
    assert_eq!(
        client.ok(&["db", "info"])["description"],
        "PUBLIC database description"
    );
    assert_eq!(
        client
            .ok(&["history", "group", "list", groupid])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let history = client.ok(&["history", "database", "list"]);
    assert_eq!(history.as_array().unwrap().len(), 2);
    let revision = history[0]["id"].as_str().unwrap();
    assert!(
        !client
            .run(
                &["history", "database", "purge", "--revision", revision],
                Some(PASSWORD)
            )
            .status
            .success()
    );
    client.ok(&[
        "history",
        "database",
        "purge",
        "--revision",
        revision,
        "--yes",
        "--operation",
        "PUBLIC db purge",
    ]);
    assert_eq!(
        client
            .ok(&["history", "database", "list"])
            .as_array()
            .unwrap()
            .len(),
        1
    );
    let local = client.ok(&["draft", "create", "--username", "PUBLIC unfinished"]);
    let localid = identity(&local).0;
    client.ok(&["draft", "update", "--draft", localid, "--title", ""]);
    let old = client.file.clone().unwrap();
    let occupied = directory.path().join("PUBLIC-occupied.taypeer");
    std::fs::write(&occupied, b"PUBLIC unrelated").unwrap();
    assert!(
        !client
            .run(
                &["db", "relocate", occupied.to_str().unwrap()],
                Some(PASSWORD)
            )
            .status
            .success()
    );
    assert_eq!(std::fs::read(&occupied).unwrap(), b"PUBLIC unrelated");
    let destination = directory.path().join("PUBLIC-relocated.taypeer");
    let relocated = client.ok(&["db", "relocate", destination.to_str().unwrap()]);
    assert_eq!(
        relocated["copy"]["path"],
        destination.canonicalize().unwrap().to_str().unwrap()
    );
    assert!(!old.exists());
    client.file = Some(destination);
    let resumed = client.ok(&["draft", "resume", localid]);
    assert_eq!(resumed["fields"]["title"], "");
}

#[test]
fn immediate_trash_repeats_the_original_selection_after_process_restart() {
    let directory = tempfile::tempdir().unwrap();
    let mut client = Client::new(directory.path());
    client.create();
    let group = client.ok(&["group", "create", "--name", "PUBLIC subtree"]);
    let group = group["id"].as_str().unwrap();
    let entry = client.ok(&[
        "entry",
        "create",
        "--group",
        group,
        "--title",
        "PUBLIC entry",
    ]);
    let entry = entry.as_str().unwrap();
    let args = [
        "group",
        "trash",
        group,
        "--operation",
        "PUBLIC immediate trash",
    ];
    let receipt = client.ok(&args);
    let bytes = std::fs::read(client.file.as_ref().unwrap()).unwrap();
    assert_eq!(client.ok(&args), receipt);
    assert_eq!(std::fs::read(client.file.as_ref().unwrap()).unwrap(), bytes);
    assert_eq!(client.ok(&["entry", "list"]), json!([]));
    assert_eq!(client.ok(&["trash", "list"]).as_array().unwrap().len(), 2);
    assert_eq!(
        client
            .ok(&["history", "list", entry])
            .as_array()
            .unwrap()
            .len(),
        1
    );
}
