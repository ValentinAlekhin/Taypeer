use ed25519_dalek::SigningKey;
use taypeer_encrypted_sync_spike::{
    Control,
    container::{Draft, Store},
    session::SecretSession,
};
use std::fs;
#[test]
fn lock_reaps_secret_process_even_if_draft_save_fails() {
    for fail in [false, true] {
        let dir = tempfile::tempdir().unwrap();
        let key = SigningKey::from_bytes(&[1; 32]);
        let id = key.verifying_key().to_bytes();
        let root = Control {
            group: [7; 16],
            seq: 0,
            epoch: 1,
            manager: id,
            members: [id].into(),
            previous: [0; 32],
        };
        let root_file = dir.path().join("root");
        fs::write(&root_file, serde_json::to_vec(&root.hash()).unwrap()).unwrap();
        let pw = b"PUBLIC synthetic spike password";
        let s = Store::create(dir.path().join("database"), root, pw, &key).unwrap();
        let draft = dir.path().join("draft");
        let worker = SecretSession::open(
            &test_binary("secret-worker", env!("CARGO_BIN_EXE_secret-worker")),
            &s.path,
            &root_file,
            &draft,
            pw,
            fail,
        )
        .unwrap();
        let pid = worker.pid();
        assert_eq!(worker.lock(b"PUBLIC draft fixture").unwrap(), !fail);
        // SAFETY: signal 0 probes only the PID of our reaped child, never sends a signal.
        assert_eq!(unsafe { libc::kill(pid as i32, 0) }, -1);
        if !fail {
            assert_eq!(
                Draft::open(&s.load().unwrap().unlock(pw).unwrap(), &draft)
                    .unwrap()
                    .as_slice(),
                b"PUBLIC draft fixture"
            );
        } else {
            assert!(!draft.exists());
        }
        assert!(s.load().unwrap().queue.is_empty());
    }
}

fn test_binary(name: &str, built: &str) -> std::path::PathBuf {
    std::env::var_os("TAYPEER_TEST_BIN_DIR")
        .map(|dir| std::path::PathBuf::from(dir).join(name))
        .unwrap_or_else(|| built.into())
}
