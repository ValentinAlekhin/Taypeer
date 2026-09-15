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

#[test]
#[ignore = "native-keychain: explicitly opt in; may display macOS access dialogs"]
fn binary_commands_and_receipts_survive_new_processes() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let input = directory.path().join("PUBLIC-input.txt");
    let output = directory.path().join("PUBLIC-output.txt");
    let (doc, group, _, entry) = support::fixture(&path);
    support::persist(&path, &doc, PASSWORD);
    fs::write(&input, b"PUBLIC original").unwrap();
    let add = [
        "attachment",
        "add",
        "--entry",
        entry.as_str(),
        input.to_str().unwrap(),
        "--operation",
        "PUBLIC add",
    ];
    ok(&path, &add);
    let committed = fs::read(&path).unwrap();
    fs::remove_file(&input).unwrap();
    ok(&path, &add); // Receipt is checked before attempting to reopen the missing source.
    assert_eq!(fs::read(&path).unwrap(), committed);
    let view = ok(&path, &["attachment", "list", "--entry", entry.as_str()]);
    let attachment = view["attachments"][0]["id"].as_str().unwrap();
    let blob = view["attachments"][0]["contents"][0]["id"]
        .as_str()
        .unwrap();
    let history = ok(&path, &["history", "list", entry.as_str()]);
    let revision = history.as_array().unwrap().last().unwrap()["id"]
        .as_str()
        .unwrap();
    ok(
        &path,
        &[
            "attachment",
            "rename",
            "--entry",
            entry.as_str(),
            attachment,
            "--name",
            "PUBLIC renamed",
        ],
    );
    fs::write(&input, b"PUBLIC replacement").unwrap();
    ok(
        &path,
        &[
            "attachment",
            "replace",
            "--entry",
            entry.as_str(),
            attachment,
            input.to_str().unwrap(),
        ],
    );
    ok(
        &path,
        &[
            "attachment",
            "export",
            "--entry",
            entry.as_str(),
            "--revision",
            revision,
            blob,
            "--output",
            output.to_str().unwrap(),
        ],
    );
    assert_eq!(fs::read(&output).unwrap(), b"PUBLIC original");
    assert!(
        !run(
            &path,
            &[
                "attachment",
                "export",
                "--entry",
                entry.as_str(),
                "--revision",
                revision,
                blob,
                "--output",
                path.to_str().unwrap(),
                "--overwrite"
            ]
        )
        .status
        .success()
    );
    assert!(
        !run(
            &path,
            &[
                "attachment",
                "export",
                "--entry",
                entry.as_str(),
                "--revision",
                revision,
                blob,
                "--output",
                output.to_str().unwrap()
            ]
        )
        .status
        .success()
    );
    ok(
        &path,
        &["icon", "set", "--group", group.as_str(), "folder-open"],
    );
    ok(
        &path,
        &["icon", "set", "--entry", entry.as_str(), "key-round"],
    );
    ok(
        &path,
        &[
            "appearance",
            "set",
            "--entry",
            entry.as_str(),
            "--foreground",
            "#123456",
        ],
    );
    let shown = ok(&path, &["icon", "show", "--entry", entry.as_str()]);
    assert_eq!(
        shown["icons"][0],
        serde_json::json!({"kind":"lucide", "value":"key-round"})
    );
    assert_eq!(shown["foreground"][0], serde_json::json!([18, 52, 86, 255]));
    assert!(!shown.to_string().contains("PUBLIC_HIDDEN_PROCESS"));
    ok(
        &path,
        &[
            "attachment",
            "remove",
            "--entry",
            entry.as_str(),
            attachment,
        ],
    );
    assert!(
        ok(&path, &["attachment", "list", "--entry", entry.as_str()])["attachments"]
            .as_array()
            .unwrap()
            .is_empty()
    );
    let usage = ok(&path, &["storage", "usage"]);
    assert_eq!(
        usage["attachment_bytes"],
        (b"PUBLIC original".len() + b"PUBLIC replacement".len()) as u64
    ); // Original and replacement retained by history.
    ok(&path, &["storage", "gc", "--operation", "PUBLIC gc"]);
}

#[test]
#[ignore = "native-keychain: explicitly opt in; may display macOS access dialogs"]
fn favicon_retry_does_not_repeat_network_after_worker_restart() {
    use std::{
        io::Read,
        net::TcpListener,
        sync::{
            Arc,
            atomic::{AtomicBool, AtomicUsize, Ordering},
        },
        time::Duration,
    };
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let (doc, _, _, entry) = support::fixture(&path);
    support::persist(&path, &doc, PASSWORD);
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    listener.set_nonblocking(true).unwrap();
    let url = format!("http://{}/page", listener.local_addr().unwrap());
    let count = Arc::new(AtomicUsize::new(0));
    let stopped = Arc::new(AtomicBool::new(false));
    let requests = count.clone();
    let stop = stopped.clone();
    let server = std::thread::spawn(move || {
        while !stop.load(Ordering::Relaxed) {
            match listener.accept() {
                Ok((mut socket, _)) => {
                    socket
                        .set_read_timeout(Some(Duration::from_secs(2)))
                        .unwrap();
                    let mut request = Vec::new();
                    let mut byte = [0];
                    while request.len() < 8192 && !request.ends_with(b"\r\n\r\n") {
                        if socket.read(&mut byte).unwrap_or(0) == 0 {
                            break;
                        }
                        request.push(byte[0]);
                    }
                    requests.fetch_add(1, Ordering::Relaxed);
                    let body = if request.starts_with(b"GET /page ") {
                        r#"<link rel="icon" href="/PUBLIC.svg">"#
                    } else {
                        r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><circle cx="12" cy="12" r="8"/></svg>"#
                    };
                    let _ = write!(
                        socket,
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                        body.len(),
                        body
                    );
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5))
                }
                Err(_) => break,
            }
        }
    });
    let args = [
        "icon",
        "favicon",
        "--entry",
        entry.as_str(),
        "--url",
        &url,
        "--operation",
        "PUBLIC favicon",
    ];
    let first = run(&path, &args);
    let committed = fs::read(&path).unwrap();
    let second = run(&path, &args);
    stopped.store(true, Ordering::Relaxed);
    server.join().unwrap();
    assert!(
        first.status.success(),
        "{}",
        String::from_utf8_lossy(&first.stderr)
    );
    assert!(
        second.status.success(),
        "{}",
        String::from_utf8_lossy(&second.stderr)
    );
    assert_eq!(count.load(Ordering::Relaxed), 2);
    assert_eq!(fs::read(&path).unwrap(), committed);
    let bad = run(
        &path,
        &[
            "icon",
            "url",
            "--entry",
            entry.as_str(),
            "http://PUBLIC:PUBLIC@localhost",
            "--operation",
            "PUBLIC failure",
        ],
    );
    assert!(!bad.status.success());
    assert_eq!(fs::read(&path).unwrap(), committed);
}
