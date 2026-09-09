//! Real ENOSPC / read-only filesystem probe, only for a disposable mounted test volume.
use ed25519_dalek::SigningKey;
use taypeer_encrypted_sync_spike::{Control, Fault, container::Store};
use std::{fs, io::Write, path::PathBuf};
fn main() {
    let args: Vec<_> = std::env::args().collect();
    let directory = PathBuf::from(&args[1]);
    let path = directory.join("database");
    let manager = SigningKey::from_bytes(&[1; 32]);
    let id = manager.verifying_key().to_bytes();
    let root = Control {
        group: [7; 16],
        seq: 0,
        epoch: 1,
        manager: id,
        members: [id].into(),
        previous: [0; 32],
    };
    let store = if path.exists() {
        Store::at(path, root.hash())
    } else {
        Store::create(path, root, b"PUBLIC synthetic spike password", &manager).unwrap()
    };
    let before = fs::read(&store.path).unwrap();
    if args[2] == "full" {
        let mut filler = fs::File::create(directory.join("filler")).unwrap();
        let chunk = [0u8; 4096];
        // Never run on a real host volume: explicit 64 MiB cap exceeds only the test image.
        let mut full = false;
        for _ in 0..16384 {
            if let Err(error) = filler.write_all(&chunk) {
                assert_eq!(error.raw_os_error(), Some(libc::ENOSPC));
                full = true;
                break;
            }
        }
        assert!(full, "test volume was not filled within bounded probe");
    }
    assert!(
        store
            .edit(
                b"PUBLIC synthetic spike password",
                &manager,
                &["field".into()],
                "PUBLIC unsaved",
                Fault::None
            )
            .is_err()
    );
    assert_eq!(fs::read(&store.path).unwrap(), before);
    println!(
        "mode={} rejected_write=true previous_container_unchanged=true",
        args[2]
    );
}
