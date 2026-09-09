#![cfg(feature = "network")]
use ed25519_dalek::SigningKey;
use taypeer_encrypted_sync_spike::{Control, Fault, Id, container::Store};
use std::{
    fs,
    path::Path,
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};
const PASSWORD: &[u8] = b"PUBLIC synthetic spike password";
fn key(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn id(n: u8) -> Id {
    key(n).verifying_key().to_bytes()
}
struct Process(Child);
impl Drop for Process {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}
fn wait_file(path: &Path, child: &mut Process) {
    let start = Instant::now();
    while !path.exists() {
        assert!(
            child.0.try_wait().unwrap().is_none(),
            "child exited before rendezvous"
        );
        assert!(
            start.elapsed() < Duration::from_secs(20),
            "rendezvous timeout"
        );
        thread::sleep(Duration::from_millis(20));
    }
}
fn listen(store: &Store, root: &Path, who: u8, addr: &Path, relay: &str, fault: &str) -> Process {
    let mut child = Process(
        Command::new(test_binary("sync-node", env!("CARGO_BIN_EXE_sync-node")))
            .args([
                "listen",
                store.path.to_str().unwrap(),
                root.to_str().unwrap(),
                &who.to_string(),
                addr.to_str().unwrap(),
                relay,
                fault,
            ])
            .stdout(Stdio::null())
            .spawn()
            .unwrap(),
    );
    wait_file(addr, &mut child);
    child
}
fn send(mode: &str, store: &Store, root: &Path, who: u8, addr: &Path, relay: &str) -> bool {
    Command::new(test_binary("sync-node", env!("CARGO_BIN_EXE_sync-node")))
        .args([
            mode,
            store.path.to_str().unwrap(),
            root.to_str().unwrap(),
            &who.to_string(),
            addr.to_str().unwrap(),
            relay,
            "none",
        ])
        .stdout(Stdio::null())
        .status()
        .unwrap()
        .success()
}
fn scenario(relay_only: bool) {
    let dir = tempfile::tempdir().unwrap();
    let root = Control {
        group: [7; 16],
        seq: 0,
        epoch: 1,
        manager: id(1),
        members: [id(1), id(2), id(3)].into(),
        previous: [0; 32],
    };
    let pinned = root.hash();
    let root_path = dir.path().join("root");
    fs::write(&root_path, serde_json::to_vec(&pinned).unwrap()).unwrap();
    let a = Store::create(dir.path().join("a"), root, PASSWORD, &key(1)).unwrap();
    let b = Store::at(dir.path().join("b"), pinned);
    let c = Store::at(dir.path().join("c"), pinned);
    fs::write(&b.path, a.export(id(2)).unwrap()).unwrap();
    fs::write(&c.path, a.export(id(3)).unwrap()).unwrap();
    a.edit(
        PASSWORD,
        &key(1),
        &["synthetic-field".to_string()],
        "PUBLIC network fixture",
        Fault::None,
    )
    .unwrap();
    let mut relay_guard = None;
    let relay = if relay_only {
        let url_file = dir.path().join("relay-url");
        let mut child = Process(
            Command::new(test_binary("sync-node", env!("CARGO_BIN_EXE_sync-node")))
                .args(["relay", url_file.to_str().unwrap()])
                .spawn()
                .unwrap(),
        );
        wait_file(&url_file, &mut child);
        let url = fs::read_to_string(url_file).unwrap();
        relay_guard = Some(child);
        url
    } else {
        "direct".to_string()
    };
    for (index, mode, fault, expected) in [
        (0, "truncate", "none", false),
        (1, "send", "before-rename", false),
        (2, "send", "after-rename", false),
        (3, "send", "none", true),
        (4, "send", "none", true),
    ] {
        let addr = dir.path().join(format!("b-addr-{index}"));
        let mut child = listen(&b, &root_path, 2, &addr, &relay, fault);
        assert_eq!(send(mode, &a, &root_path, 1, &addr, &relay), expected);
        assert!(child.0.wait().unwrap().success());
    }
    assert_eq!(b.load().unwrap().queue.len(), 1);
    assert!(
        b.load()
            .unwrap()
            .unlock(PASSWORD)
            .unwrap()
            .applied
            .is_empty()
    );
    let addr = dir.path().join("c-addr");
    let mut child = listen(&c, &root_path, 3, &addr, &relay, "none");
    assert!(send("send", &b, &root_path, 2, &addr, &relay));
    assert!(child.0.wait().unwrap().success());
    c.apply(PASSWORD, &key(3), Fault::None).unwrap();
    assert_eq!(c.load().unwrap().unlock(PASSWORD).unwrap().applied.len(), 1);
    // Real TLS identity (9) is not the admitted sender (1). No caller-supplied peer IDs.
    let addr = dir.path().join("reject-addr");
    let mut child = listen(&c, &root_path, 3, &addr, &relay, "none");
    assert!(!send("send", &a, &root_path, 9, &addr, &relay));
    assert!(child.0.wait().unwrap().success());
    if !relay_only {
        let addr = dir.path().join("unreachable-addr");
        let _child = listen(&b, &root_path, 2, &addr, "direct", "none");
        assert!(!send("unreachable", &a, &root_path, 1, &addr, "direct"));
    }

    // Recipient consent is signed while unlocked; old authority is durably gone
    // before network delivery. A failed delivery never reactivates the old manager.
    let consent = taypeer_encrypted_sync_spike::container::HandoffConsent::sign(
        a.load().unwrap().chain.head(),
        &key(2),
        true,
    )
    .unwrap();
    assert!(
        a.handoff(PASSWORD, &key(1), &consent, Fault::AfterRename)
            .is_err()
    );
    a.handoff(PASSWORD, &key(1), &consent, Fault::None).unwrap();
    assert!(
        a.rotate(
            PASSWORD,
            b"PUBLIC new password",
            &key(1),
            [id(1), id(2), id(3)].into(),
            Fault::None
        )
        .is_err()
    );
    let addr = dir.path().join("handoff-addr");
    let mut child = listen(&b, &root_path, 2, &addr, &relay, "none");
    assert!(send("send", &a, &root_path, 1, &addr, &relay));
    assert!(child.0.wait().unwrap().success());
    b.rotate(
        PASSWORD,
        b"PUBLIC new password",
        &key(2),
        [id(1), id(2), id(3)].into(),
        Fault::None,
    )
    .unwrap();
    drop(relay_guard);
}
#[test]
fn direct_process_exchange_faults_restart_forwarding_and_no_relay_failure() {
    scenario(false);
}
#[test]
fn forced_local_relay_process_exchange_faults_and_forwarding() {
    scenario(true);
}

#[test]
fn invitation_recipient_is_the_actual_quic_identity() {
    let dir = tempfile::tempdir().unwrap();
    let root = Control {
        group: [7; 16],
        seq: 0,
        epoch: 1,
        manager: id(1),
        members: [id(1)].into(),
        previous: [0; 32],
    };
    let pinned = root.hash();
    let root_path = dir.path().join("root");
    fs::write(&root_path, serde_json::to_vec(&pinned).unwrap()).unwrap();
    let s = Store::create(dir.path().join("a"), root, PASSWORD, &key(1)).unwrap();
    let code = s.invite(PASSWORD, &key(1), id(9), 0, Fault::None).unwrap();
    let invitation = dir.path().join("invitation");
    fs::write(&invitation, serde_json::to_vec(&code).unwrap()).unwrap();
    for (i, who, expected) in [(0, 8, false), (1, 9, true), (2, 9, false)] {
        let addr = dir.path().join(format!("addr-{i}"));
        let mut child = Process(
            Command::new(test_binary("sync-node", env!("CARGO_BIN_EXE_sync-node")))
                .args([
                    "admit-listen",
                    s.path.to_str().unwrap(),
                    root_path.to_str().unwrap(),
                    "1",
                    addr.to_str().unwrap(),
                    "direct",
                    "approved",
                ])
                .spawn()
                .unwrap(),
        );
        wait_file(&addr, &mut child);
        let status = Command::new(test_binary("sync-node", env!("CARGO_BIN_EXE_sync-node")))
            .args([
                "redeem",
                s.path.to_str().unwrap(),
                root_path.to_str().unwrap(),
                &who.to_string(),
                addr.to_str().unwrap(),
                "direct",
                invitation.to_str().unwrap(),
            ])
            .status()
            .unwrap();
        assert_eq!(status.success(), expected);
        assert!(child.0.wait().unwrap().success());
    }
    assert!(s.load().unwrap().chain.head().members.contains(&id(9)));
    assert!(!s.load().unwrap().chain.head().members.contains(&id(8)));
}

fn test_binary(name: &str, built: &str) -> std::path::PathBuf {
    std::env::var_os("TAYPEER_TEST_BIN_DIR")
        .map(|dir| std::path::PathBuf::from(dir).join(name))
        .unwrap_or_else(|| built.into())
}
