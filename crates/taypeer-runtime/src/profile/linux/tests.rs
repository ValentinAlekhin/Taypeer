//! Public credentials exercise enrollment binding and historical-key rewrapping without sockets.
use super::*;
use taypeer_services::DatabaseService;
use taypeer_sync::{Backend, Coordinator, CoordinatorPersistence, EndpointAddr};
use taypeer_trust::{InvitationSecret, JoinProof};

#[test]
fn incorrect_initial_join_password_can_be_corrected_before_epoch_binding() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC manager.taypeer");
    let author = AuthorKey::from_seed(&[51; 32]);
    let transport = Arc::new(TransportKey::from_seed(&[71; 32]));
    let identity = Identity::new(author.public(), transport.public()).unwrap();
    let password = b"PUBLIC correct database password";
    let wrong = b"PUBLIC initially incorrect password";
    let seed = DatabaseService::prepare_managed(
        "PUBLIC enrollment".into(),
        password,
        &author,
        identity,
        1,
        taypeer_core::DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap(),
    )
    .unwrap();
    let database = seed.controls[0].body.database.clone();
    let coordinator = Arc::new(Coordinator::new(Arc::clone(&transport)));
    coordinator
        .register(seed.create(&path, &transport, None).unwrap())
        .unwrap();
    let port = CoordinatorPersistence::new(
        Arc::clone(&coordinator),
        database.clone(),
        path.clone(),
        Digest::of(b"PUBLIC manager copy"),
    )
    .unwrap();
    let mut service = DatabaseService::new();
    let session = service
        .open_managed(Box::new(port), password, || {
            Ok(Some(AuthorKey::from_seed(&[51; 32])))
        })
        .unwrap();
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let (invitation, secret): (_, InvitationSecret) =
        service.create_invitation(&session, now).unwrap();
    let lease = NativeProfile::acquire(&directory.path().join("PUBLIC recipient profile")).unwrap();
    let profile = lease.profile();
    let (recipient, capability) = profile
        .prepare_join_credentials(&invitation, wrong)
        .unwrap();
    let recipient_transport = TransportKey::from_seed(&capability.transport_seed);
    let recipient_identity =
        Identity::new(recipient.public(), recipient_transport.public()).unwrap();
    let proof = JoinProof::sign(&invitation, recipient_identity, &recipient).unwrap();
    coordinator
        .command(
            recipient_transport.public(),
            taypeer_sync::Command::Join {
                invitation: Box::new(invitation.clone()),
                secret: Zeroizing::new(*secret.expose()),
                proof: Box::new(proof),
                address: EndpointAddr::new(
                    recipient_transport.public().to_string().parse().unwrap(),
                ),
            },
        )
        .unwrap();
    service
        .approve_invitation(&session, invitation.id().unwrap(), now + 1)
        .unwrap();
    let snapshot = coordinator.snapshot(&database).unwrap();
    assert!(DatabaseService::authenticate_archive_credentials(&snapshot, wrong).is_err());
    let epoch = DatabaseService::authenticate_archive_credentials(&snapshot, password).unwrap();
    // The original staging authentication stays in the enrollment worker while
    // the corrected database password provides the independently authenticated epoch.
    profile
        .bind_join_credentials(&snapshot, wrong, &epoch)
        .unwrap();
    let history = DatabaseService::authenticate_archive_keyring(&snapshot, password).unwrap();
    let (loaded, loaded_capability) = profile
        .database_credentials(
            &database,
            invitation.root,
            snapshot.chain().head().epoch,
            &epoch,
            &history,
        )
        .unwrap();
    assert_eq!(loaded.unwrap().public(), recipient.public());
    assert_eq!(
        TransportKey::from_seed(&loaded_capability.transport_seed).public(),
        recipient_transport.public()
    );

    // A remaining recipient learns a rotation after restart and uses authenticated
    // historical keys solely to rewrap its local author object under the current epoch.
    let next = b"PUBLIC next database password";
    service
        .rotate_password(&session, Digest::of(b"PUBLIC rotation"), next, None)
        .unwrap();
    let snapshot = coordinator.snapshot(&database).unwrap();
    let epoch = DatabaseService::authenticate_archive_credentials(&snapshot, next).unwrap();
    let history = DatabaseService::authenticate_archive_keyring(&snapshot, next).unwrap();
    let (loaded, _) = profile
        .database_credentials(
            &database,
            invitation.root,
            snapshot.chain().head().epoch,
            &epoch,
            &history,
        )
        .unwrap();
    profile
        .finalize_authenticated_credentials(
            &database,
            invitation.root,
            &epoch,
            snapshot.chain().head().epoch,
        )
        .unwrap();
    assert_eq!(loaded.unwrap().public(), recipient.public());
}
