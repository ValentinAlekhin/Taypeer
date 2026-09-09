use super::*;
use tempfile::TempDir;
const PASSWORD: &[u8] = b"PUBLIC synthetic spike password";
const NEW: &[u8] = b"PUBLIC new password";
fn signer(n: u8) -> SigningKey {
    SigningKey::from_bytes(&[n; 32])
}
fn id(n: u8) -> Id {
    signer(n).verifying_key().to_bytes()
}
fn members(ns: &[u8]) -> BTreeSet<Id> {
    ns.iter().map(|n| id(*n)).collect()
}
fn path(names: &[&str]) -> Vec<String> {
    names.iter().map(|n| n.to_string()).collect()
}
fn setup() -> (TempDir, Store) {
    let dir = tempfile::tempdir().unwrap();
    let root = Control {
        group: [7; 16],
        seq: 0,
        epoch: 1,
        manager: id(1),
        members: members(&[1, 2, 3]),
        previous: [0; 32],
    };
    let store = Store::create(dir.path().join("database"), root, PASSWORD, &signer(1)).unwrap();
    (dir, store)
}
fn copy(store: &Store, dir: &TempDir, name: &str) -> Store {
    let p = dir.path().join(name);
    fs::write(&p, store.export(id(2)).unwrap()).unwrap();
    Store::at(p, store.pinned)
}
fn packet(
    state: &State,
    doc: &mut Automerge,
    who: u8,
    names: &[&str],
    value: &str,
) -> (ChangeHash, Packet) {
    doc.set_actor(actor(&signer(who)));
    let mut tx = doc.transaction();
    put_path(&mut tx, &path(names), value).unwrap();
    let (hash, _) = tx.commit();
    let hash = hash.unwrap();
    (
        hash,
        Packet::create(state, &signer(who), &doc.get_change_by_hash(&hash).unwrap()).unwrap(),
    )
}
fn values(doc: &Automerge, names: &[&str]) -> Vec<String> {
    let (last, parents) = names.split_last().unwrap();
    let mut obj = ROOT;
    for name in parents {
        obj = doc.get(&obj, *name).unwrap().unwrap().1;
    }
    doc.get_all(obj, *last)
        .unwrap()
        .iter()
        .map(|(v, _)| v.to_str().unwrap().to_string())
        .collect()
}
fn state(store: &Store, password: &[u8]) -> State {
    store.load().unwrap().unlock(password).unwrap()
}
#[test]
fn several_rotations_latest_password_preserves_conflicts_and_intermediate_history() {
    let (dir, manager) = setup();
    let phone = copy(&manager, &dir, "phone");
    let initial = state(&phone, PASSWORD);
    let old_snapshot = phone.load().unwrap().checkpoint.sealed;
    let mut offline = initial.doc().unwrap();
    let (first, p1) = packet(&initial, &mut offline, 2, &["field"], "offline-first");
    let (second, p2) = packet(&initial, &mut offline, 2, &["field"], "offline-second");
    phone.receive(id(2), p2, Fault::None).unwrap();
    phone.receive(id(2), p1, Fault::None).unwrap();
    manager
        .edit(
            PASSWORD,
            &signer(1),
            &path(&["field"]),
            "online",
            Fault::None,
        )
        .unwrap();
    manager
        .rotate(
            PASSWORD,
            b"PUBLIC intermediate",
            &signer(1),
            members(&[1, 2]),
            Fault::None,
        )
        .unwrap();
    manager
        .rotate(
            b"PUBLIC intermediate",
            NEW,
            &signer(1),
            members(&[1, 2]),
            Fault::None,
        )
        .unwrap();
    phone
        .receive_container(id(1), &manager.export(id(2)).unwrap(), Fault::None)
        .unwrap();
    assert!(phone.load().unwrap().unlock(PASSWORD).is_err());
    let report = phone.apply(NEW, &signer(2), Fault::None).unwrap();
    assert_eq!(report[&first], Transfer::AutoApply);
    assert_eq!(report[&second], Transfer::AutoApply);
    let current = state(&phone, NEW);
    assert_eq!(current.keys.len(), 3);
    let doc = current.doc().unwrap();
    assert_eq!(values(&doc, &["field"]).len(), 2);
    assert_eq!(
        values(&doc.fork_at(&[first]).unwrap(), &["field"]),
        ["offline-first"]
    );
    assert_eq!(doc.get_change_by_hash(&second).unwrap().deps(), &[first]);
    assert!(old_snapshot.open_key(current.keys.get(&1).unwrap()).is_ok());
    assert!(
        phone
            .load()
            .unwrap()
            .checkpoint
            .sealed
            .open_key(initial.keys.get(&1).unwrap())
            .is_err()
    );
    assert!(
        phone
            .apply(NEW, &signer(2), Fault::None)
            .unwrap()
            .values()
            .all(|s| *s == Transfer::AlreadyApplied)
    );
}
#[test]
fn revoked_dependency_closure_is_quarantined_while_independent_work_continues() {
    let (_dir, store) = setup();
    let s = state(&store, PASSWORD);
    let mut bad = s.doc().unwrap();
    let (created, p1) = packet(
        &s,
        &mut bad,
        3,
        &["missing-entry", "field"],
        "revoked-created",
    );
    let (dependent, p2) = packet(
        &s,
        &mut bad,
        2,
        &["missing-entry", "field"],
        "trusted-edited",
    );
    let (transitive, p3) = packet(&s, &mut bad, 1, &["another"], "transitive");
    let mut independent = s.doc().unwrap();
    let (good, p4) = packet(&s, &mut independent, 2, &["safe"], "allowed");
    store
        .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    for p in [p1, p2, p3, p4] {
        store.receive(id(2), p, Fault::None).unwrap();
    }
    let report = store.apply(NEW, &signer(1), Fault::None).unwrap();
    for h in [created, dependent, transitive] {
        assert_eq!(report[&h], Transfer::NeedsReview);
    }
    assert_eq!(report[&good], Transfer::AutoApply);
    let doc = state(&store, NEW).doc().unwrap();
    assert!(doc.get(ROOT, "missing-entry").unwrap().is_none());
    assert!(doc.get(ROOT, "another").unwrap().is_none());
    assert_eq!(values(&doc, &["safe"]), ["allowed"]);
    assert_eq!(store.load().unwrap().queue.len(), 4);
}
#[test]
fn interrupted_admission_cannot_be_hidden_by_readmission_or_reencryption() {
    let (_dir, store) = setup();
    let s = state(&store, PASSWORD);
    let (hash, p) = packet(&s, &mut s.doc().unwrap(), 2, &["field"], "old");
    store
        .rotate(PASSWORD, NEW, &signer(1), members(&[1, 3]), Fault::None)
        .unwrap();
    store
        .rotate(NEW, PASSWORD, &signer(1), members(&[1, 2, 3]), Fault::None)
        .unwrap();
    store.receive(id(1), p.clone(), Fault::None).unwrap();
    let mut forged = p;
    forged.control = store.load().unwrap().chain.head().hash();
    assert!(store.receive(id(1), forged, Fault::None).is_err());
    assert_eq!(
        store.inspect(PASSWORD).unwrap()[&hash],
        Transfer::NeedsReview
    );
}
#[test]
fn missing_dependencies_wait_then_apply_without_duplicate_changes() {
    let (_dir, store) = setup();
    let s = state(&store, PASSWORD);
    let mut doc = s.doc().unwrap();
    let (h1, p1) = packet(&s, &mut doc, 2, &["field"], "one");
    let (h2, p2) = packet(&s, &mut doc, 2, &["field"], "two");
    store.receive(id(2), p2.clone(), Fault::None).unwrap();
    assert_eq!(
        store.apply(PASSWORD, &signer(1), Fault::None).unwrap()[&h2],
        Transfer::AwaitingDependencies
    );
    store.receive(id(2), p1, Fault::None).unwrap();
    store.receive(id(2), p2, Fault::None).unwrap();
    let report = store.apply(PASSWORD, &signer(1), Fault::None).unwrap();
    assert_eq!(report[&h1], Transfer::AutoApply);
    assert_eq!(report[&h2], Transfer::AutoApply);
    let fresh = state(&store, PASSWORD);
    let original = fresh.doc().unwrap().get_change_by_hash(&h2).unwrap();
    store
        .receive(
            id(2),
            Packet::create(&fresh, &signer(2), &original).unwrap(),
            Fault::None,
        )
        .unwrap();
    assert!(
        store
            .apply(PASSWORD, &signer(1), Fault::None)
            .unwrap()
            .values()
            .all(|s| *s == Transfer::AlreadyApplied)
    );
    assert_eq!(
        state(&store, PASSWORD)
            .doc()
            .unwrap()
            .get_changes(&[])
            .len(),
        2
    );
}
#[test]
fn manual_extraction_is_new_signed_revision_with_origin_and_does_not_restore_admission() {
    let (dir, store) = setup();
    let s = state(&store, PASSWORD);
    let mut doc = s.doc().unwrap();
    let (source, p) = packet(&s, &mut doc, 3, &["entry", "field"], "selected");
    store.receive(id(3), p, Fault::None).unwrap();
    store
        .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    let peer = copy(&store, &dir, "peer");
    let request = Extraction {
        operation: [5; 32],
        source,
        choices: vec![Selection {
            path: path(&["entry", "field"]),
            variant: 0,
        }],
    };
    assert_eq!(
        store.review(NEW, source, &request.choices).unwrap()[0].to_str(),
        Some("selected")
    );
    let hash = store
        .extract(NEW, &signer(1), request.clone(), Fault::None)
        .unwrap();
    assert_ne!(hash, source);
    assert_eq!(
        store
            .extract(NEW, &signer(1), request.clone(), Fault::None)
            .unwrap(),
        hash
    );
    let current = state(&store, NEW);
    let doc = current.doc().unwrap();
    assert!(doc.get_change_by_hash(&source).is_none());
    let change = doc.get_change_by_hash(&hash).unwrap();
    assert_eq!(change.actor_id().to_bytes(), id(1));
    let provenance: Provenance = serde_json::from_str(change.message().unwrap()).unwrap();
    assert!(provenance.origins.contains(&(id(3), source.0)));
    assert_eq!(store.inspect(NEW).unwrap()[&source], Transfer::NeedsReview);
    assert!(store.export(id(3)).is_err());
    peer.receive_container(id(1), &store.export(id(2)).unwrap(), Fault::None)
        .unwrap();
    peer.apply(NEW, &signer(2), Fault::None).unwrap();
    assert_eq!(
        values(&state(&peer, NEW).doc().unwrap(), &["entry", "field"]),
        ["selected"]
    );
    assert_eq!(
        peer.extract(NEW, &signer(2), request, Fault::None).unwrap(),
        hash
    );
}
#[test]
fn backup_restore_preserves_current_password_control_and_synchronizes_new_operations() {
    let (dir, store) = setup();
    store
        .edit(
            PASSWORD,
            &signer(1),
            &path(&["entry", "field"]),
            "backup-value",
            Fault::None,
        )
        .unwrap();
    let backup = store.load().unwrap();
    store
        .edit(
            PASSWORD,
            &signer(1),
            &path(&["entry", "field"]),
            "current-value",
            Fault::None,
        )
        .unwrap();
    store
        .edit(
            PASSWORD,
            &signer(1),
            &path(&["later"]),
            "remove-on-restore",
            Fault::None,
        )
        .unwrap();
    store
        .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    let peer = copy(&store, &dir, "peer");
    let before = store.load().unwrap();
    let hash = store
        .restore(NEW, &signer(1), [6; 32], &backup, Fault::None)
        .unwrap();
    assert_eq!(
        store
            .restore(NEW, &signer(1), [6; 32], &backup, Fault::None)
            .unwrap(),
        hash
    );
    let after = store.load().unwrap();
    assert_eq!(before.chain.head().hash(), after.chain.head().hash());
    assert!(after.unlock(PASSWORD).is_err());
    assert!(!after.chain.head().members.contains(&id(3)));
    let doc = after.unlock(NEW).unwrap().doc().unwrap();
    assert_eq!(values(&doc, &["entry", "field"]), ["backup-value"]);
    assert!(doc.get(ROOT, "later").unwrap().is_none());
    assert_eq!(
        doc.get_change_by_hash(&hash).unwrap().actor_id().to_bytes(),
        id(1)
    );
    let pre_path = store
        .path
        .with_extension(format!("before-restore-{}", hex(&[6; 32])));
    let pre = Container::parse(&fs::read(pre_path).unwrap(), store.pinned).unwrap();
    assert_eq!(
        values(
            &pre.unlock(NEW).unwrap().doc().unwrap(),
            &["entry", "field"]
        ),
        ["current-value"]
    );
    peer.receive_container(id(1), &store.export(id(2)).unwrap(), Fault::None)
        .unwrap();
    peer.apply(NEW, &signer(2), Fault::None).unwrap();
    assert_eq!(
        values(&state(&peer, NEW).doc().unwrap(), &["entry", "field"]),
        ["backup-value"]
    );
}
#[test]
fn all_write_faults_reopen_and_retry_without_false_ack_or_duplicate_recovery() {
    for fault in [
        Fault::BeforeWrite,
        Fault::PartialWrite,
        Fault::BeforeRename,
        Fault::AfterRename,
        Fault::BeforeDirectorySync,
    ] {
        let (_dir, store) = setup();
        let s = state(&store, PASSWORD);
        let (source, p) = packet(&s, &mut s.doc().unwrap(), 3, &["field"], "recover");
        assert!(store.receive(id(3), p.clone(), fault).is_err());
        let store = Store::at(store.path, store.pinned);
        store.receive(id(3), p, Fault::None).unwrap();
        assert_eq!(store.load().unwrap().queue.len(), 1);
        store
            .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
            .unwrap();
        let req = Extraction {
            operation: [6; 32],
            source,
            choices: vec![Selection {
                path: path(&["field"]),
                variant: 0,
            }],
        };
        assert!(store.extract(NEW, &signer(1), req.clone(), fault).is_err());
        let store = Store::at(store.path, store.pinned);
        let hash = store
            .extract(NEW, &signer(1), req.clone(), Fault::None)
            .unwrap();
        assert_eq!(
            store.extract(NEW, &signer(1), req, Fault::None).unwrap(),
            hash
        );
        assert_eq!(state(&store, NEW).doc().unwrap().get_changes(&[]).len(), 1);
    }
}
#[test]
fn apply_and_restore_faults_keep_document_and_operation_ids_atomic() {
    for fault in [
        Fault::BeforeWrite,
        Fault::PartialWrite,
        Fault::BeforeRename,
        Fault::AfterRename,
        Fault::BeforeDirectorySync,
    ] {
        let (_dir, store) = setup();
        let s = state(&store, PASSWORD);
        let (h, p) = packet(&s, &mut s.doc().unwrap(), 2, &["field"], "one");
        let backup = store.load().unwrap();
        store.receive(id(2), p, Fault::None).unwrap();
        assert!(store.apply(PASSWORD, &signer(1), fault).is_err());
        let store = Store::at(store.path, store.pinned);
        store.apply(PASSWORD, &signer(1), Fault::None).unwrap();
        let s = state(&store, PASSWORD);
        assert!(s.applied.contains(&h.0));
        assert_eq!(s.doc().unwrap().get_changes(&[]).len(), 1);
        assert!(
            store
                .restore(PASSWORD, &signer(1), [4; 32], &backup, fault)
                .is_err()
        );
        let store = Store::at(store.path, store.pinned);
        let restored = store
            .restore(PASSWORD, &signer(1), [4; 32], &backup, Fault::None)
            .unwrap();
        assert_eq!(
            store
                .restore(PASSWORD, &signer(1), [4; 32], &backup, Fault::None)
                .unwrap(),
            restored
        );
        assert_eq!(
            state(&store, PASSWORD)
                .doc()
                .unwrap()
                .get_changes(&[])
                .len(),
            2
        );
    }
}
#[test]
fn locked_container_forwarding_and_failed_latest_import_keep_offline_queue() {
    let (dir, a) = setup();
    let b = copy(&a, &dir, "b");
    let c = copy(&a, &dir, "c");
    let s = state(&b, PASSWORD);
    let (h, p) = packet(&s, &mut s.doc().unwrap(), 2, &["phone"], "offline");
    b.receive(id(2), p, Fault::None).unwrap();
    a.rotate(PASSWORD, NEW, &signer(1), members(&[1, 2, 3]), Fault::None)
        .unwrap();
    let bytes = a.export(id(2)).unwrap();
    assert!(
        b.receive_container(id(1), &bytes, Fault::BeforeRename)
            .is_err()
    );
    b.receive_container(id(1), &bytes, Fault::None).unwrap();
    c.receive_container(id(2), &b.export(id(3)).unwrap(), Fault::None)
        .unwrap();
    assert!(
        state(&c, NEW)
            .doc()
            .unwrap()
            .get(ROOT, "phone")
            .unwrap()
            .is_none()
    );
    assert_eq!(
        c.apply(NEW, &signer(3), Fault::None).unwrap()[&h],
        Transfer::AutoApply
    );
    assert_eq!(c.load().unwrap().queue.len(), 1);
}
#[test]
fn signed_control_fork_halts_exchange_and_management_across_restart_but_reading_and_recovery_work()
{
    let (dir, a) = setup();
    let b = copy(&a, &dir, "b");
    a.edit(
        PASSWORD,
        &signer(1),
        &path(&["field"]),
        "readable",
        Fault::None,
    )
    .unwrap();
    a.rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    b.rotate(PASSWORD, NEW, &signer(1), members(&[1, 3]), Fault::None)
        .unwrap();
    assert!(
        a.receive_container(id(1), &b.export(id(1)).unwrap(), Fault::None)
            .is_err()
    );
    let a = Store::at(a.path, a.pinned);
    assert!(a.export(id(1)).is_err());
    assert!(
        a.rotate(NEW, PASSWORD, &signer(1), members(&[1]), Fault::None)
            .is_err()
    );
    assert!(a.apply(NEW, &signer(1), Fault::None).is_err());
    assert_eq!(
        values(&state(&a, NEW).doc().unwrap(), &["field"]),
        ["readable"]
    );
    let recovered = a
        .recover_trust(NEW, PASSWORD, &signer(9), dir.path().join("recovered"))
        .unwrap();
    assert_eq!(
        recovered.load().unwrap().chain.head().members,
        members(&[9])
    );
    assert_ne!(recovered.load().unwrap().chain.head().group, [7; 16]);
    assert!(recovered.export(id(1)).is_err());
    assert_eq!(
        values(&state(&recovered, PASSWORD).doc().unwrap(), &["field"]),
        ["readable"]
    );
}
#[test]
fn invitation_binds_database_recipient_five_minutes_and_atomic_consumption() {
    let (_dir, store) = setup();
    let code = store
        .invite(PASSWORD, &signer(1), id(9), 10, Fault::None)
        .unwrap();
    assert_eq!(code.token.len(), 32);
    assert!(code.qr_payload().unwrap().contains(&hex(&id(9))));
    let request = |code: InvitationCode, peer, now, approved| Redemption {
        code,
        peer,
        now,
        approved,
    };
    assert!(
        store
            .redeem(
                PASSWORD,
                &signer(1),
                request(code.clone(), id(9), 310, true),
                Fault::None
            )
            .is_err()
    );
    assert!(
        store
            .redeem(
                PASSWORD,
                &signer(1),
                request(code.clone(), id(2), 11, true),
                Fault::None
            )
            .is_err()
    );
    let mut wrong = code.clone();
    wrong.group = [8; 16];
    assert!(
        store
            .redeem(
                PASSWORD,
                &signer(1),
                request(wrong, id(9), 11, true),
                Fault::None
            )
            .is_err()
    );
    assert!(
        store
            .redeem(
                PASSWORD,
                &signer(1),
                request(code.clone(), id(9), 11, true),
                Fault::BeforeRename
            )
            .is_err()
    );
    assert!(!store.load().unwrap().chain.head().members.contains(&id(9)));
    store
        .redeem(
            PASSWORD,
            &signer(1),
            request(code.clone(), id(9), 12, true),
            Fault::None,
        )
        .unwrap();
    assert!(store.load().unwrap().chain.head().members.contains(&id(9)));
    assert!(
        store
            .redeem(
                PASSWORD,
                &signer(1),
                request(code, id(9), 13, true),
                Fault::None
            )
            .is_err()
    );
    let denied = store
        .invite(PASSWORD, &signer(1), id(8), 20, Fault::None)
        .unwrap();
    assert!(
        store
            .redeem(
                PASSWORD,
                &signer(1),
                request(denied.clone(), id(8), 21, false),
                Fault::None
            )
            .is_err()
    );
    assert!(
        store
            .redeem(
                PASSWORD,
                &signer(1),
                request(denied, id(8), 22, true),
                Fault::None
            )
            .is_err()
    );
    assert!(!store.load().unwrap().chain.head().members.contains(&id(8)));
}
#[test]
fn tamper_unknown_versions_control_binding_and_foreign_backup_do_not_overwrite() {
    let (_dir, store) = setup();
    let before = fs::read(&store.path).unwrap();
    let mut c = store.load().unwrap();
    c.version = 99;
    assert!(Container::parse(&c.bytes().unwrap(), store.pinned).is_err());
    c = store.load().unwrap();
    c.checkpoint.sealed.content.bytes[0] ^= 1;
    assert!(
        store
            .receive_container(id(2), &c.bytes().unwrap(), Fault::None)
            .is_err()
    );
    c = store.load().unwrap();
    c.checkpoint.sealed.control = [9; 32];
    assert!(
        store
            .receive_container(id(2), &c.bytes().unwrap(), Fault::None)
            .is_err()
    );
    assert!(Container::parse(&vec![0; MAX_BYTES + 1], store.pinned).is_err());
    assert_eq!(fs::read(&store.path).unwrap(), before);
}

#[test]
fn accepted_history_survives_latest_state_but_remote_retained_snapshots_do_not_bypass_review() {
    let (dir, manager) = setup();
    let phone = copy(&manager, &dir, "phone");
    let accepted = phone
        .edit(
            PASSWORD,
            &signer(3),
            &path(&["accepted"]),
            "local-before-revoke",
            Fault::None,
        )
        .unwrap();
    manager
        .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    let peer = copy(&manager, &dir, "peer");
    phone
        .receive_container(id(1), &manager.export(id(2)).unwrap(), Fault::None)
        .unwrap();
    assert!(
        state(&phone, NEW)
            .doc()
            .unwrap()
            .get_change_by_hash(&accepted)
            .is_some()
    );
    // Forwarded queue does not confer this phone's local acceptance on another peer.
    peer.receive_container(id(2), &phone.export(id(1)).unwrap(), Fault::None)
        .unwrap();
    assert_eq!(
        peer.apply(NEW, &signer(1), Fault::None).unwrap()[&accepted],
        Transfer::NeedsReview
    );
    assert_eq!(
        phone.apply(NEW, &signer(2), Fault::None).unwrap()[&accepted],
        Transfer::AlreadyApplied
    );
    assert_eq!(
        values(&state(&phone, NEW).doc().unwrap(), &["accepted"]),
        ["local-before-revoke"]
    );
    assert!(phone.load().unwrap().retained.is_empty());
}
#[test]
fn rotation_fault_retry_and_pre_rotation_backup_never_mixes_keys_and_control() {
    for fault in [
        Fault::BeforeWrite,
        Fault::PartialWrite,
        Fault::BeforeRename,
        Fault::AfterRename,
        Fault::BeforeDirectorySync,
    ] {
        let (_dir, store) = setup();
        assert!(
            store
                .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), fault)
                .is_err()
        );
        let store = Store::at(store.path, store.pinned);
        let current = store.load().unwrap();
        if current.chain.head().epoch == 1 {
            current.unlock(PASSWORD).unwrap();
            store
                .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
                .unwrap();
        } else {
            assert_eq!(current.chain.head().epoch, 2);
            current.unlock(NEW).unwrap();
        }
        let before = Container::parse(
            &fs::read(store.path.with_extension("before-rotation-1")).unwrap(),
            store.pinned,
        )
        .unwrap();
        assert_eq!(before.unlock(PASSWORD).unwrap().epoch, 1);
        assert_eq!(state(&store, NEW).keys.len(), 2);
    }
}
#[test]
fn backup_conflicts_and_reused_operation_ids_require_explicit_selection() {
    let (_dir, store) = setup();
    let s = state(&store, PASSWORD);
    for who in [2, 3] {
        let (_, p) = packet(
            &s,
            &mut s.doc().unwrap(),
            who,
            &["field"],
            if who == 2 { "left" } else { "right" },
        );
        store.receive(id(who), p, Fault::None).unwrap();
    }
    store.apply(PASSWORD, &signer(1), Fault::None).unwrap();
    let backup = store.load().unwrap();
    let before = backup.bytes().unwrap();
    assert!(
        store
            .restore(PASSWORD, &signer(1), [7; 32], &backup, Fault::None)
            .is_err()
    );
    assert_eq!(fs::read(&store.path).unwrap(), before);
    let source = backup.queue[0].change(&s).unwrap().hash();
    let req = Extraction {
        operation: [7; 32],
        source,
        choices: vec![Selection {
            path: path(&["field"]),
            variant: 0,
        }],
    };
    store
        .extract(PASSWORD, &signer(1), req.clone(), Fault::None)
        .unwrap();
    let mut reused = req;
    reused.source = backup.queue[1].change(&s).unwrap().hash();
    assert!(
        store
            .extract(PASSWORD, &signer(1), reused, Fault::None)
            .is_err()
    );
}
#[test]
fn fork_evidence_from_a_future_branch_is_durable_and_failed_persistence_latches_current_session() {
    let (dir, a) = setup();
    let b = copy(&a, &dir, "b");
    let old = copy(&a, &dir, "old");
    a.rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    b.rotate(PASSWORD, NEW, &signer(1), members(&[1, 3]), Fault::None)
        .unwrap();
    assert!(
        a.receive_container(id(1), &b.export(id(1)).unwrap(), Fault::None)
            .is_err()
    );
    let evidence = a.load().unwrap().bytes().unwrap();
    assert!(
        old.receive_container(id(1), &evidence, Fault::BeforeRename)
            .is_err()
    );
    assert!(old.export(id(1)).is_err());
    let old = Store::at(old.path, old.pinned);
    assert!(
        old.receive_container(id(1), &evidence, Fault::None)
            .is_err()
    );
    let old = Store::at(old.path, old.pinned);
    assert!(old.export(id(1)).is_err());
    state(&old, PASSWORD);
}

#[test]
fn encountering_a_signed_fork_in_a_backup_also_halts_the_store() {
    let (dir, a) = setup();
    let b = copy(&a, &dir, "b");
    a.rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    b.rotate(PASSWORD, NEW, &signer(1), members(&[1, 3]), Fault::None)
        .unwrap();
    assert!(
        a.restore(NEW, &signer(1), [2; 32], &b.load().unwrap(), Fault::None)
            .is_err()
    );
    let a = Store::at(a.path, a.pinned);
    assert!(a.export(id(1)).is_err());
    state(&a, NEW);
}
#[test]
fn invitation_uncertain_commit_persists_consumption_and_admission_together() {
    let (_dir, store) = setup();
    let code = store
        .invite(PASSWORD, &signer(1), id(9), 0, Fault::None)
        .unwrap();
    let request = Redemption {
        code: code.clone(),
        peer: id(9),
        now: 1,
        approved: true,
    };
    assert!(
        store
            .redeem(PASSWORD, &signer(1), request, Fault::AfterRename)
            .is_err()
    );
    let store = Store::at(store.path, store.pinned);
    assert!(store.load().unwrap().chain.head().members.contains(&id(9)));
    let request = Redemption {
        code,
        peer: id(9),
        now: 2,
        approved: true,
    };
    assert!(
        store
            .redeem(PASSWORD, &signer(1), request, Fault::None)
            .is_err()
    );
}
#[test]
fn foreign_backup_rejected_and_revoked_credentials_cannot_sign_current_changes() {
    let (dir, store) = setup();
    let foreign = store
        .recover_trust(PASSWORD, NEW, &signer(9), dir.path().join("foreign"))
        .unwrap();
    let before = fs::read(&store.path).unwrap();
    assert!(
        store
            .restore(
                PASSWORD,
                &signer(1),
                [3; 32],
                &foreign.load().unwrap(),
                Fault::None
            )
            .is_err()
    );
    assert_eq!(fs::read(&store.path).unwrap(), before);
    store
        .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    assert!(
        store
            .edit(NEW, &signer(3), &path(&["field"]), "no", Fault::None)
            .is_err()
    );
    assert!(store.apply(NEW, &signer(3), Fault::None).is_err());
}

#[test]
fn member_snapshot_cannot_smuggle_unreviewed_revoked_history_into_a_new_epoch() {
    let (dir, manager) = setup();
    let old = state(&manager, PASSWORD);
    let mut doc = old.doc().unwrap();
    let (revoked, p) = packet(&old, &mut doc, 3, &["forged-accepted"], "must-review");
    let receiver = copy(&manager, &dir, "receiver");
    manager
        .rotate(PASSWORD, NEW, &signer(1), members(&[1, 2]), Fault::None)
        .unwrap();
    let mut forged = manager.load().unwrap();
    let mut state = forged.unlock(NEW).unwrap();
    let mut forged_doc = state.doc().unwrap();
    forged_doc
        .apply_changes([p.change(&state).unwrap()])
        .unwrap();
    state.document = forged_doc.save();
    state.applied.insert(revoked.0);
    forged.checkpoint = Checkpoint::create(&state, NEW, &signer(2)).unwrap();
    forged.queue.push(p);
    receiver
        .receive_container(id(2), &forged.bytes().unwrap(), Fault::None)
        .unwrap();
    assert_eq!(
        receiver.inspect(NEW).unwrap()[&revoked],
        Transfer::NeedsReview
    );
    assert!(
        receiver
            .load()
            .unwrap()
            .unlock(NEW)
            .unwrap()
            .doc()
            .unwrap()
            .get(ROOT, "forged-accepted")
            .unwrap()
            .is_none()
    );
    forged.base = Some(forged.checkpoint.clone());
    assert!(Container::parse(&forged.bytes().unwrap(), receiver.pinned).is_err());
}
#[test]
fn explicit_v2_migration_preserves_original_and_requires_current_manager() {
    let (dir, store) = setup();
    let mut legacy = store.load().unwrap();
    legacy.version = 2;
    legacy.base = None;
    let bytes = legacy.bytes().unwrap();
    let path = dir.path().join("legacy");
    fs::write(&path, &bytes).unwrap();
    let migration = Store::at(path, store.pinned);
    assert!(migration.load().is_err());
    assert!(
        migration
            .migrate_v2(PASSWORD, &signer(2), Fault::None)
            .is_err()
    );
    assert_eq!(fs::read(&migration.path).unwrap(), bytes);
    assert!(
        migration
            .migrate_v2(PASSWORD, &signer(1), Fault::BeforeRename)
            .is_err()
    );
    migration
        .migrate_v2(PASSWORD, &signer(1), Fault::None)
        .unwrap();
    assert_eq!(migration.load().unwrap().version, 3);
    assert_eq!(
        fs::read(migration.path.with_extension("before-migration-2")).unwrap(),
        bytes
    );
    migration.load().unwrap().unlock(PASSWORD).unwrap();
}
