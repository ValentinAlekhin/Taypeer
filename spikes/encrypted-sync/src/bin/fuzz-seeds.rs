//! Generates public synthetic parser seeds; no user files are read.
use automerge::{Automerge, ROOT, ReadDoc, transaction::Transactable};
fn main() {
    use taypeer_encrypted_sync_spike::{Control, container::Store};
    let dir = std::path::PathBuf::from(std::env::args().nth(1).unwrap());
    std::fs::create_dir_all(&dir).unwrap();
    let mut doc = Automerge::new();
    let mut tx = doc.transaction();
    tx.put(ROOT, "field", "PUBLIC fuzz fixture").unwrap();
    let (hash, _) = tx.commit();
    std::fs::write(dir.join("automerge-document"), doc.save()).unwrap();
    std::fs::write(
        dir.join("automerge-change"),
        doc.get_change_by_hash(&hash.unwrap()).unwrap().raw_bytes(),
    )
    .unwrap();
    let key = ed25519_dalek::SigningKey::from_bytes(&[1; 32]);
    let id = key.verifying_key().to_bytes();
    let root = Control {
        group: [7; 16],
        seq: 0,
        epoch: 1,
        manager: id,
        members: [id].into(),
        previous: [0; 32],
    };
    let temporary = std::env::temp_dir().join(format!("taypeer-fuzz-seed-{}", std::process::id()));
    std::fs::create_dir(&temporary).unwrap();
    let store = Store::create(
        temporary.join("database"),
        root,
        b"PUBLIC fuzz password",
        &key,
    )
    .unwrap();
    std::fs::write(
        dir.join("container-v3"),
        store.load().unwrap().bytes().unwrap(),
    )
    .unwrap();
    std::fs::remove_dir_all(temporary).unwrap();
}
