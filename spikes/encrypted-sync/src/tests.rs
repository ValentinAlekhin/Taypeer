use super::*;
use automerge::{ActorId, Automerge, ROOT, ReadDoc, transaction::Transactable};
use tempfile::TempDir;
const PASSWORD: &[u8] = b"PUBLIC synthetic spike password";
fn signer(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn setup() -> (Snapshot, LockedInbox, TempDir) {
    let dir = tempfile::tempdir().unwrap();
    let snapshot = Snapshot {
        group: [7; 16],
        epoch: 1,
        read_key: [8; 32],
        document: Automerge::new().save(),
        applied: BTreeSet::new(),
    };
    let inbox = LockedInbox {
        group: snapshot.group,
        epoch: 1,
        members: [1, 2, 3]
            .map(|n| signer(n).verifying_key().to_bytes())
            .into(),
        directory: dir.path().to_owned(),
    };
    (snapshot, inbox, dir)
}
fn edits(s: &Snapshot, who: u8, values: &[&str]) -> Vec<Envelope> {
    let mut doc = Automerge::load(&s.document).unwrap();
    let signer = signer(who);
    doc.set_actor(ActorId::from(signer.verifying_key().to_bytes().to_vec()));
    values
        .iter()
        .map(|value| {
            let mut tx = doc.transaction();
            tx.put(ROOT, "synthetic-field", *value).unwrap();
            let (hash, _) = tx.commit();
            Envelope::create(
                s.group,
                s.epoch,
                &s.read_key,
                &signer,
                doc.get_change_by_hash(&hash.unwrap()).unwrap().raw_bytes(),
            )
        })
        .collect()
}
fn checkpoint(dir: &TempDir) -> std::path::PathBuf {
    dir.path().join("checkpoint")
}
#[test]
fn locked_b_forwards_to_c_then_unlock_and_restart() {
    let (s, b, _bd) = setup();
    let (mut c_s, c, cd) = setup();
    let e = edits(&s, 1, &["synthetic-value"]).remove(0);
    let a = signer(1).verifying_key().to_bytes();
    let b_id = signer(2).verifying_key().to_bytes();
    b.receive(a, &e.encode(), Fault::None).unwrap();
    for bytes in b.pending() {
        c.receive(b_id, &bytes, Fault::None).unwrap();
    }
    assert_eq!(Automerge::load(&c_s.document).unwrap().length(ROOT), 0);
    assert_eq!(
        apply_pending(&c, &mut c_s, &checkpoint(&cd), PASSWORD, Fault::None),
        Ok(1)
    );
    let restored = open(&fs::read(checkpoint(&cd)).unwrap(), PASSWORD).unwrap();
    assert_eq!(Automerge::load(&restored.document).unwrap().length(ROOT), 1);
    assert_eq!(restored.applied.len(), 1);
}
#[test]
fn reordering_duplicates_and_missing_dependencies() {
    let (mut s, inbox, dir) = setup();
    let es = edits(&s, 1, &["one", "two"]);
    inbox
        .receive(es[1].author, &es[1].encode(), Fault::None)
        .unwrap();
    assert_eq!(
        apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None),
        Ok(0)
    );
    inbox
        .receive(es[0].author, &es[0].encode(), Fault::None)
        .unwrap();
    inbox
        .receive(es[1].author, &es[1].encode(), Fault::None)
        .unwrap();
    assert_eq!(inbox.pending().len(), 2);
    assert_eq!(
        apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None),
        Ok(2)
    );
    assert_eq!(
        apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None),
        Ok(0)
    );
    let mut restored = open(&fs::read(checkpoint(&dir)).unwrap(), PASSWORD).unwrap();
    assert_eq!(
        apply_pending(
            &inbox,
            &mut restored,
            &checkpoint(&dir),
            PASSWORD,
            Fault::None
        ),
        Ok(0)
    );
}
#[test]
fn concurrent_automerge_values_survive_encrypted_delivery() {
    let (mut s, inbox, dir) = setup();
    for who in [1, 2] {
        let e = edits(&s, who, &[if who == 1 { "left" } else { "right" }]).remove(0);
        inbox.receive(e.author, &e.encode(), Fault::None).unwrap();
    }
    assert_eq!(
        apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None),
        Ok(2)
    );
    let doc = Automerge::load(&s.document).unwrap();
    assert_eq!(doc.get_all(ROOT, "synthetic-field").unwrap().len(), 2);
}
#[test]
fn disk_faults_never_ack_and_retry_is_idempotent() {
    for fault in [
        Fault::BeforeWrite,
        Fault::PartialWrite,
        Fault::BeforeRename,
        Fault::AfterRename,
        Fault::BeforeDirectorySync,
    ] {
        let (s, inbox, _dir) = setup();
        let e = edits(&s, 1, &["one"]).remove(0);
        assert!(inbox.receive(e.author, &e.encode(), fault).is_err());
        inbox.receive(e.author, &e.encode(), Fault::None).unwrap();
        inbox.receive(e.author, &e.encode(), Fault::None).unwrap();
        assert_eq!(inbox.pending().len(), 1);
    }
}
#[test]
fn failed_apply_preserves_previous_checkpoint_and_can_restart() {
    let (mut s, inbox, dir) = setup();
    let old = seal(&s, PASSWORD).unwrap();
    atomic_save(&checkpoint(&dir), &old, Fault::None).unwrap();
    let e = edits(&s, 1, &["one"]).remove(0);
    inbox.receive(e.author, &e.encode(), Fault::None).unwrap();
    assert!(
        apply_pending(
            &inbox,
            &mut s,
            &checkpoint(&dir),
            PASSWORD,
            Fault::BeforeRename
        )
        .is_err()
    );
    assert!(s.applied.is_empty());
    assert_eq!(fs::read(checkpoint(&dir)).unwrap(), old);
    assert_eq!(
        apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None),
        Ok(1)
    );
    assert!(
        open(
            &fs::read(checkpoint(&dir).with_extension("previous")).unwrap(),
            PASSWORD
        )
        .unwrap()
        .applied
        .is_empty()
    );
}
#[test]
fn file_tamper_wrong_password_version_and_kdf_bombs() {
    let (s, _, _dir) = setup();
    let bytes = seal(&s, PASSWORD).unwrap();
    assert!(open(&bytes, b"wrong").is_err());
    let mut b = bytes.clone();
    b[8] = 9;
    assert!(matches!(open(&b, PASSWORD), Err("version")));
    let mut b = bytes.clone();
    b[10..14].copy_from_slice(&u32::MAX.to_le_bytes());
    assert!(matches!(open(&b, PASSWORD), Err("KDF policy")));
    let mut b = bytes.clone();
    b[14..18].copy_from_slice(&0u32.to_le_bytes());
    assert!(matches!(open(&b, PASSWORD), Err("KDF policy")));
    let mut b = bytes.clone();
    b[18] ^= 1;
    assert!(open(&b, PASSWORD).is_err());
    let mut b = bytes.clone();
    *b.last_mut().unwrap() ^= 1;
    assert!(open(&b, PASSWORD).is_err());
    assert!(open(&vec![0; MAX_BYTES + 1], PASSWORD).is_err());
    for len in [0, 8, 58, 73] {
        assert!(open(&bytes[..len], PASSWORD).is_err());
    }
}
#[test]
fn signed_packets_still_require_author_admission_and_valid_content() {
    let (mut s, inbox, dir) = setup();
    let e = edits(&s, 1, &["one"]).remove(0);
    let mut b = e.clone();
    b.ciphertext[0] ^= 1;
    assert!(inbox.receive(e.author, &b.encode(), Fault::None).is_err());
    let mut b = e.clone();
    b.group = [9; 16];
    assert!(inbox.receive(e.author, &b.encode(), Fault::None).is_err());
    assert!(
        inbox
            .receive(
                signer(9).verifying_key().to_bytes(),
                &e.encode(),
                Fault::None
            )
            .is_err()
    );
    let invalid = Envelope::create(
        s.group,
        s.epoch,
        &s.read_key,
        &signer(1),
        b"not an Automerge change",
    );
    inbox
        .receive(invalid.author, &invalid.encode(), Fault::None)
        .unwrap();
    assert!(apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None).is_err());
    assert!(s.applied.is_empty());
}
#[test]
fn receive_before_revoke_is_not_apply_before_revoke() {
    let (mut s, mut inbox, dir) = setup();
    let es = edits(&s, 2, &["accepted-history", "queued-late"]);
    inbox
        .receive(es[0].author, &es[0].encode(), Fault::None)
        .unwrap();
    assert_eq!(
        apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None),
        Ok(1)
    );
    inbox
        .receive(es[1].author, &es[1].encode(), Fault::None)
        .unwrap();
    inbox.members.remove(&es[0].author);
    inbox.epoch = 2;
    s.epoch = 2;
    s.read_key = [9; 32];
    assert!(
        inbox
            .receive(
                signer(1).verifying_key().to_bytes(),
                &es[1].encode(),
                Fault::None
            )
            .is_err()
    );
    assert_eq!(
        apply_pending(&inbox, &mut s, &checkpoint(&dir), PASSWORD, Fault::None),
        Ok(0)
    );
    assert_eq!(s.applied.len(), 1);
    assert_eq!(inbox.pending().len(), 2);
    let doc = Automerge::load(&s.document).unwrap();
    assert_eq!(doc.get_all(ROOT, "synthetic-field").unwrap().len(), 1);
}
#[test]
fn two_rotations_old_snapshot_and_new_set_recovery() {
    let (mut s, inbox, _dir) = setup();
    let old = seal(&s, PASSWORD).unwrap();
    s.epoch = 3;
    s.read_key = [10; 32];
    let new = seal(&s, b"PUBLIC new password").unwrap();
    assert!(open(&new, PASSWORD).is_err());
    assert!(open(&old, PASSWORD).is_ok());
    let e = edits(&s, 1, &["future"]).remove(0);
    assert!(e.decrypt(&[8; 32]).is_err());
    let copied = open(&new, b"PUBLIC new password").unwrap();
    assert_eq!(copied.epoch, 3);
    assert!(
        inbox
            .receive(
                signer(9).verifying_key().to_bytes(),
                &e.encode(),
                Fault::None
            )
            .is_err()
    );
    let mut recovered = copied;
    recovered.group = [11; 16];
    recovered.epoch = 1;
    recovered.read_key = [12; 32];
    recovered.applied.clear();
    let target = LockedInbox {
        group: recovered.group,
        epoch: 1,
        members: [signer(9).verifying_key().to_bytes()].into(),
        directory: inbox.directory,
    };
    assert!(target.receive(e.author, &e.encode(), Fault::None).is_err());
}
fn control() -> Control {
    Control {
        group: [7; 16],
        seq: 0,
        epoch: 1,
        manager: signer(1).verifying_key().to_bytes(),
        members: [1, 2, 3]
            .map(|n| signer(n).verifying_key().to_bytes())
            .into(),
        previous: [0; 32],
    }
}
#[test]
fn signed_control_chain_rotation_replay_and_wrong_manager() {
    let c = control();
    let mut next = c.clone();
    next.seq += 1;
    next.previous = c.hash();
    next.epoch += 1;
    next.members.remove(&signer(2).verifying_key().to_bytes());
    assert!(c.sign_successor(&next, &signer(2), true).is_err());
    assert!(c.sign_successor(&next, &signer(1), false).is_err());
    let sig = c.sign_successor(&next, &signer(1), true).unwrap();
    c.accept(&next, &sig).unwrap();
    assert!(next.accept(&next, &sig).is_err());
    next.epoch = 1;
    let sig = c.sign_successor(&next, &signer(1), true).unwrap();
    assert!(c.accept(&next, &sig).is_err());
}
#[test]
fn transfer_failure_has_zero_active_then_recovery_has_one() {
    let dir = tempfile::tempdir().unwrap();
    let old_path = dir.path().join("old-control");
    let new_path = dir.path().join("new-control");
    let c = control();
    let mut next = c.clone();
    next.seq = 1;
    next.previous = c.hash();
    next.manager = signer(2).verifying_key().to_bytes();
    let sig = c.sign_successor(&next, &signer(1), true).unwrap();
    c.accept(&next, &sig).unwrap();
    // Candidate handoff journal: old durably relinquishes before delivering its certificate.
    // Both confirmations/unlocked checks are preconditions, not OS/UI integrations.
    atomic_save(&old_path, &serde_json::to_vec(&next).unwrap(), Fault::None).unwrap();
    assert!(
        atomic_save(
            &new_path,
            &serde_json::to_vec(&next).unwrap(),
            Fault::BeforeRename
        )
        .is_err()
    );
    let restored: Control = serde_json::from_slice(&fs::read(&old_path).unwrap()).unwrap();
    let old_active = restored.manager == signer(1).verifying_key().to_bytes();
    let new_active = new_path.exists();
    assert!(!old_active && !new_active);
    atomic_save(&new_path, &serde_json::to_vec(&next).unwrap(), Fault::None).unwrap();
    let restored_new: Control = serde_json::from_slice(&fs::read(&new_path).unwrap()).unwrap();
    assert_eq!(
        usize::from(old_active)
            + usize::from(restored_new.manager == signer(2).verifying_key().to_bytes()),
        1
    );
}
#[test]
fn rollback_fork_is_detectable_but_not_prevented_by_signatures() {
    let c = control();
    let mut a = c.clone();
    a.seq = 1;
    a.previous = c.hash();
    a.manager = signer(2).verifying_key().to_bytes();
    let mut b = a.clone();
    b.manager = signer(3).verifying_key().to_bytes();
    // Deliberate counterexample: two offline replicas can accept different successors.
    c.accept(&a, &c.sign_successor(&a, &signer(1), true).unwrap())
        .unwrap();
    c.accept(&b, &c.sign_successor(&b, &signer(1), true).unwrap())
        .unwrap();
    assert_ne!(a.hash(), b.hash());
}
#[test]
fn invitation_expiry_denial_replay_and_failed_durable_consume() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("invitation");
    let token = b"PUBLIC 256-bit synthetic token value";
    let make = || Invitation {
        token_hash: digest(token),
        expires: 300,
        consumed: false,
    };
    assert!(make().redeem(token, 300, true, &path, Fault::None).is_err());
    assert!(
        make()
            .redeem(b"wrong", 1, true, &path, Fault::None)
            .is_err()
    );
    let mut denied = make();
    assert!(denied.redeem(token, 1, false, &path, Fault::None).is_err());
    assert!(denied.redeem(token, 2, true, &path, Fault::None).is_err());
    let mut inv = make();
    assert!(
        inv.redeem(token, 1, true, &path, Fault::BeforeRename)
            .is_err()
    );
    assert!(!inv.consumed);
    inv.redeem(token, 2, true, &path, Fault::None).unwrap();
    let mut restored: Invitation = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    assert!(
        restored
            .redeem(token, 3, true, dir.path(), Fault::None)
            .is_err()
    );
}

#[test]
fn rfc8032_ed25519_vector_1() {
    // RFC 8032 §7.1 TEST 1, public test material, empty message.
    fn bytes(s: &str) -> Vec<u8> {
        s.as_bytes()
            .chunks_exact(2)
            .map(|c| u8::from_str_radix(std::str::from_utf8(c).unwrap(), 16).unwrap())
            .collect()
    }
    let seed: [u8; 32] = bytes("9d61b19deffd5a60ba844af492ec2cc44449c5697b326919703bac031cae7f60")
        .try_into()
        .unwrap();
    let key = SigningKey::from_bytes(&seed);
    assert_eq!(
        key.verifying_key().to_bytes().to_vec(),
        bytes("d75a980182b10ab7d54bfed3c964073a0ee172f3daa62325af021a68f707511a")
    );
    let expected = bytes(
        "e5564300c360ac729086e2cc806e828a84877f1eb8e5d974d873e065224901555fb8821590a33bacc61e39701cf9b46bd25bf5f0595bbe24655141438e7a100b",
    );
    assert_eq!(key.sign(b"").to_bytes().to_vec(), expected);
    key.verifying_key()
        .verify_strict(b"", &Signature::from_slice(&expected).unwrap())
        .unwrap();
}
