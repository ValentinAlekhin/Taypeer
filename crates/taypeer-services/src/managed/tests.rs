//! All keys, passwords and contents in this module are deliberately PUBLIC synthetic fixtures.
use super::*;
use std::{path::PathBuf, sync::Arc};
use taypeer_storage::ArchiveStore;
use taypeer_sync::{Backend, Command, Coordinator, CoordinatorPersistence, Reply};
use taypeer_trust::{JoinProof, TransportKey};

const PASSWORD: &[u8] = b"PUBLIC managed fixture password";
const NEW_PASSWORD: &[u8] = b"PUBLIC independently rotated fixture password";

#[test]
fn member_checkpoint_cannot_launder_revoked_sources_or_their_dependencies() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(11);
    let b = Profile::new(12);
    let c = Profile::new(13);
    let (mut sa, ta) = create(&a, directory.path().join("a.taypeer"));
    admit(&mut sa, &ta, &a, &b);
    admit(&mut sa, &ta, &a, &c);
    let bp = directory.path().join("b.taypeer");
    let cp = directory.path().join("c.taypeer");
    copy_to(&a, &ta.database, &bp);
    copy_to(&a, &ta.database, &cp);
    let (mut sb, tb) = b.open(bp, PASSWORD);
    let (mut sc, tc) = c.open(cp, PASSWORD);
    sb.create_group(&tb, "PUBLIC revoked source".into(), None)
        .unwrap();
    sc.create_group(&tc, "PUBLIC independent source".into(), None)
        .unwrap();
    deliver(&b, &c, &ta.database);
    assert_eq!(sc.apply_received(&tc).unwrap().value.applied, 1);
    sc.create_group(&tc, "PUBLIC dependent source".into(), None)
        .unwrap();
    sa.rotate_password(
        &ta,
        Digest::of(b"PUBLIC revoke B"),
        NEW_PASSWORD,
        Some(b.identity().device),
    )
    .unwrap();
    deliver(&c, &a, &ta.database);
    let report = sa.apply_received(&ta).unwrap().value;
    assert_eq!(report.applied, 1);
    assert!(
        report
            .pending
            .iter()
            .any(|packet| packet.reason == PendingReason::RevokedAuthor)
    );
    assert!(
        report
            .pending
            .iter()
            .any(|packet| packet.reason == PendingReason::Dependency)
    );
    let groups = sa.groups(&ta).unwrap().value;
    assert_eq!(groups.len(), 1);
    assert_eq!(groups[0].name, "PUBLIC independent source");
    let before = a.coordinator.snapshot(&ta.database).unwrap().fingerprint();
    assert_eq!(sa.apply_received(&ta).unwrap().value.applied, 0);
    assert_eq!(
        before,
        a.coordinator.snapshot(&ta.database).unwrap().fingerprint()
    );
}

#[test]
fn managed_reauthentication_keeps_failed_session_locked_and_expires_old_tokens() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(10);
    let path = directory.path().join("reauthentication.taypeer");
    let (mut service, original) = create(&profile, path.clone());
    service
        .create_group(&original, "PUBLIC retained group".into(), None)
        .unwrap();
    service.lock(&original).unwrap();
    let port = || {
        Box::new(
            CoordinatorPersistence::new(
                Arc::clone(&profile.coordinator),
                original.database.clone(),
                path.clone(),
                Digest::of(&profile.seed),
            )
            .unwrap(),
        )
    };
    let failed = service.open_managed(port(), b"PUBLIC wrong password", || {
        panic!("Credentials accessed before authentication")
    });
    assert!(matches!(
        failed,
        Err(ServiceError::Storage(StorageError::Authentication))
    ));
    assert!(service.entries(&original, None, "").is_err());
    let reopened = service
        .open_managed(port(), PASSWORD, || Ok(Some(profile.author())))
        .unwrap();
    assert_ne!(reopened, original);
    assert!(service.entries(&original, None, "").is_err());
    assert_eq!(service.groups(&reopened).unwrap().value.len(), 1);
}

#[test]
fn administrative_retries_preserve_exact_intent_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(9);
    let path = directory.path().join("administration.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let operation = Digest::of(b"PUBLIC retry rotation");
    service
        .rotate_password(&session, operation, NEW_PASSWORD, None)
        .unwrap();
    let rotated = profile
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();
    service
        .rotate_password(&session, operation, NEW_PASSWORD, None)
        .unwrap();
    assert_eq!(
        rotated,
        profile
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint()
    );
    assert!(matches!(
        service.rotate_password(&session, operation, b"PUBLIC different intent", None),
        Err(ServiceError::Trust(taypeer_trust::Error::OperationMismatch))
    ));
    let policy_operation = Digest::of(b"PUBLIC retry KDF policy");
    let policy = DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 600).unwrap();
    service
        .set_database_policy(&session, policy_operation, policy, Some(NEW_PASSWORD))
        .unwrap();
    service.lock(&session).unwrap();
    drop(service);
    profile.coordinator.unregister(&session.database).unwrap();
    let (mut service, session) = profile.open(path, NEW_PASSWORD);
    let before = profile
        .coordinator
        .snapshot(&session.database)
        .unwrap()
        .fingerprint();
    service
        .set_database_policy(&session, policy_operation, policy, None)
        .unwrap();
    service
        .rotate_password(&session, operation, NEW_PASSWORD, None)
        .unwrap();
    assert_eq!(
        before,
        profile
            .coordinator
            .snapshot(&session.database)
            .unwrap()
            .fingerprint()
    );
    let other = DatabasePolicy::new(512 * 1024, 2 * 1024 * 1024, 600).unwrap();
    assert!(matches!(
        service.set_database_policy(&session, policy_operation, other, None),
        Err(ServiceError::Trust(taypeer_trust::Error::OperationMismatch))
    ));
}
struct Profile {
    seed: [u8; 32],
    transport: Arc<TransportKey>,
    coordinator: Arc<Coordinator>,
}
impl Profile {
    fn new(n: u8) -> Self {
        let transport = Arc::new(TransportKey::from_seed(&[n + 40; 32]));
        let coordinator = Arc::new(Coordinator::new(Arc::clone(&transport)));
        Self {
            seed: [n; 32],
            transport,
            coordinator,
        }
    }
    fn author(&self) -> AuthorKey {
        AuthorKey::from_seed(&self.seed)
    }
    fn identity(&self) -> Identity {
        Identity::new(self.author().public(), self.transport.public()).unwrap()
    }
    fn open(&self, path: PathBuf, password: &[u8]) -> (DatabaseService, SessionToken) {
        let snapshot = taypeer_storage::ArchiveSnapshot::open(&path, None).unwrap();
        let database = snapshot.chain().head().database.clone();
        let store =
            ArchiveStore::open(&path, Some(snapshot.chain().root().unwrap()), None).unwrap();
        self.coordinator.register(store).unwrap();
        let port = CoordinatorPersistence::new(
            Arc::clone(&self.coordinator),
            database,
            path,
            Digest::of(&self.seed),
        )
        .unwrap();
        let mut service = DatabaseService::new();
        let session = service
            .open_managed(Box::new(port), password, || Ok(Some(self.author())))
            .unwrap();
        (service, session)
    }
}
fn create(profile: &Profile, path: PathBuf) -> (DatabaseService, SessionToken) {
    let seed = DatabaseService::prepare_managed(
        "PUBLIC shared file".into(),
        PASSWORD,
        &profile.author(),
        profile.identity(),
        1,
        DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap(),
    )
    .unwrap();
    let store = seed.create(&path, &profile.transport, None).unwrap();
    drop(store);
    profile.open(path, PASSWORD)
}
fn copy_to(source: &Profile, database: &DatabaseId, destination: &std::path::Path) {
    let snapshot = source.coordinator.snapshot(database).unwrap();
    snapshot
        .candidate()
        .export(snapshot.metadata().clone(), destination)
        .unwrap();
}
fn admit(
    service: &mut DatabaseService,
    session: &SessionToken,
    manager: &Profile,
    member: &Profile,
) {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (invitation, secret) = service.create_invitation(session, now).unwrap();
    let proof = JoinProof::sign(&invitation, member.identity(), &member.author()).unwrap();
    let address =
        taypeer_sync::EndpointAddr::new(member.transport.public().to_string().parse().unwrap());
    let request = invitation.id().unwrap();
    manager
        .coordinator
        .command(
            member.transport.public(),
            Command::Join {
                invitation: Box::new(invitation),
                secret: Zeroizing::new(*secret.expose()),
                proof: Box::new(proof),
                address,
            },
        )
        .unwrap();
    assert_eq!(
        service
            .approve_invitation(session, request, now + 1)
            .unwrap(),
        member.identity().device
    );
    assert_eq!(
        service
            .approve_invitation(session, request, now + 2)
            .unwrap(),
        member.identity().device
    );
}
fn deliver(from: &Profile, to: &Profile, database: &DatabaseId) {
    let snapshot = from.coordinator.snapshot(database).unwrap();
    let reply = to
        .coordinator
        .command(
            from.transport.public(),
            Command::Offer(Box::new(snapshot.metadata().clone())),
        )
        .unwrap();
    let Reply::Needed(needed) = reply else {
        panic!("expected synthetic inventory response");
    };
    for id in needed {
        let mut temp = tempfile::NamedTempFile::new().unwrap();
        std::io::copy(&mut snapshot.reader(id).unwrap(), &mut temp).unwrap();
        to.coordinator
            .receive(
                from.transport.public(),
                database,
                snapshot.metadata().manifest.body.objects[&id].clone(),
                temp,
            )
            .unwrap();
    }
}

#[test]
fn durable_edits_copied_read_only_and_credentials_only_after_authentication() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(1);
    let path = directory.path().join("a.taypeer");
    let (mut service, session) = create(&a, path.clone());
    let group = service
        .create_group(&session, "PUBLIC group".into(), None)
        .unwrap()
        .value;
    service
        .start_create_entry(&session, group.id.clone())
        .unwrap();
    service
        .update_draft(
            &session,
            EditableEntry {
                title: "PUBLIC entry".into(),
                password: Some("PUBLIC value".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = service.save_draft(&session).unwrap().value;
    let before = a.coordinator.snapshot(&session.database).unwrap();
    let report = service.apply_received(&session).unwrap().value;
    assert_eq!(report.applied, 0);
    assert!(report.pending.is_empty());
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    assert_eq!(
        service
            .database_policy(&session)
            .unwrap()
            .value
            .attachment_bytes(),
        1024 * 1024
    );

    let copied = directory.path().join("copy.taypeer");
    copy_to(&a, &session.database, &copied);
    let b = Profile::new(2);
    let (mut reader, read_session) = b.open(copied.clone(), PASSWORD);
    assert!(!reader.can_write(&read_session).unwrap());
    assert_eq!(
        reader.entries(&read_session, None, "").unwrap().value.len(),
        1
    );
    assert!(matches!(
        reader.create_group(&read_session, "PUBLIC forbidden".into(), None),
        Err(ServiceError::ReadOnly)
    ));
    reader.lock(&read_session).unwrap();
    drop(reader);
    b.coordinator.unregister(&session.database).unwrap();

    let store = ArchiveStore::open(&copied, None, None).unwrap();
    b.coordinator.register(store).unwrap();
    let port = CoordinatorPersistence::new(
        Arc::clone(&b.coordinator),
        session.database.clone(),
        copied,
        Digest::of(b"PUBLIC copy marker"),
    )
    .unwrap();
    let called = std::cell::Cell::new(false);
    let result = DatabaseService::new().open_managed(Box::new(port), b"PUBLIC incorrect", || {
        called.set(true);
        Ok(Some(b.author()))
    });
    assert!(matches!(
        result,
        Err(ServiceError::Storage(StorageError::Authentication))
    ));
    assert!(!called.get());
    assert!(
        before
            .metadata()
            .manifest
            .body
            .objects
            .values()
            .any(|o| o.kind == ObjectKind::Change)
    );
}

#[test]
fn three_members_merge_offline_history_forward_locked_and_keep_drafts_through_rotation() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(4);
    let b = Profile::new(5);
    let c = Profile::new(6);
    let a_path = directory.path().join("a.taypeer");
    let b_path = directory.path().join("b.taypeer");
    let c_path = directory.path().join("c.taypeer");
    let (mut sa, ta) = create(&a, a_path);
    let group = sa
        .create_group(&ta, "PUBLIC group".into(), None)
        .unwrap()
        .value;
    sa.start_create_entry(&ta, group.id.clone()).unwrap();
    sa.update_draft(
        &ta,
        EditableEntry {
            title: "PUBLIC entry".into(),
            password: Some("PUBLIC base".into()),
            ..Default::default()
        },
    )
    .unwrap();
    let entry = sa.save_draft(&ta).unwrap().value;
    admit(&mut sa, &ta, &a, &b);
    admit(&mut sa, &ta, &a, &c);
    copy_to(&a, &ta.database, &b_path);
    copy_to(&a, &ta.database, &c_path);
    let (mut sb, tb) = b.open(b_path.clone(), PASSWORD);
    let (mut sc, tc) = c.open(c_path.clone(), PASSWORD);
    sa.start_edit_entry(&ta, &entry).unwrap();
    sb.start_edit_entry(&tb, &entry).unwrap();
    let mut fa = sa.draft(&ta).unwrap().value.unwrap().fields;
    let mut fb = sb.draft(&tb).unwrap().value.unwrap().fields;
    fa.password = Some("PUBLIC offline A".into());
    fb.password = Some("PUBLIC offline B".into());
    sa.update_draft(&ta, fa).unwrap();
    sb.update_draft(&tb, fb).unwrap();
    sa.save_draft(&ta).unwrap();
    sb.save_draft(&tb).unwrap();
    sc.lock(&tc).unwrap();
    deliver(&a, &c, &ta.database);
    deliver(&c, &b, &ta.database);
    let report = sb.apply_received(&tb).unwrap().value;
    assert_eq!(report.applied, 1);
    assert!(report.pending.is_empty());
    assert_eq!(sb.history(&tb, &entry).unwrap().value.len(), 3);
    assert!(sb.view_entry(&tb, &entry).unwrap().value.has_conflicts);
    assert_eq!(sb.apply_received(&tb).unwrap().value.applied, 0);

    sb.start_create_entry(&tb, group.id).unwrap();
    sb.update_draft(
        &tb,
        EditableEntry {
            title: "PUBLIC interrupted draft".into(),
            ..Default::default()
        },
    )
    .unwrap();
    sb.lock(&tb).unwrap();
    sa.rotate_password(&ta, Digest::of(b"PUBLIC rotation"), NEW_PASSWORD, None)
        .unwrap();
    deliver(&a, &b, &ta.database);
    drop(sb);
    b.coordinator.unregister(&ta.database).unwrap();
    let (mut sb, tb) = b.open(b_path, NEW_PASSWORD);
    assert!(sb.pending_draft(&tb).unwrap().value.is_some());
    assert_eq!(sb.history(&tb, &entry).unwrap().value.len(), 3);
    sb.restore_draft(&tb).unwrap();
    assert_eq!(
        sb.draft(&tb).unwrap().value.unwrap().fields.title,
        "PUBLIC interrupted draft"
    );
    sb.cancel_draft(&tb).unwrap();
}
