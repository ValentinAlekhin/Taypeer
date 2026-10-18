//! Public credentials exercise enrollment binding and historical-key rewrapping without sockets.
use super::*;
use taypeer_services::DatabaseService;
use taypeer_sync::{Backend, Coordinator, CoordinatorPersistence, EndpointAddr};
use taypeer_trust::{InvitationSecret, JoinProof};

#[test]
fn reader_state_creation_resumes_authenticated_preparation_without_authority() {
    for state_exists in [false, true] {
        let directory = tempfile::tempdir().unwrap();
        let lease = NativeProfile::acquire(directory.path()).unwrap();
        let profile = lease.profile();
        let database = DatabaseId::new("PUBLIC reader credential recovery");
        let root = Digest::of(b"PUBLIC reader root");
        let key = ReadKey::from_secret(&[57; 32]);
        let scope = LocalCredentialScope {
            database: database.clone(),
            root,
            epoch: 0,
        };
        let transport = TransportKey::from_seed(&[58; 32]);
        let local = LocalStateKey::generate().unwrap();
        let record = LocalCredential::read_only(scope.clone(), &transport, &local);
        let path = profile.database_directory(&database, root);
        private_directory(&path).unwrap();
        LocalCredentialStore::create_record(&path.join("credentials.prepared"), &key, &record)
            .unwrap();
        if state_exists {
            let (active, _) = profile
                .activate(&TransportCapability {
                    database: database.clone(),
                    root,
                    enrolled: false,
                    transport_seed: transport.secret_seed(),
                    local_state_seed: Some(Zeroizing::new(*local.secret_bytes())),
                })
                .unwrap();
            active
                .save_state("PUBLIC retained marker", &41_u64)
                .unwrap();
            drop(active);
        } else {
            std::fs::create_dir(path.join("state")).unwrap();
            assert!(
                profile
                    .database_credentials(&database, root, 0, &key, &[])
                    .is_err()
            );
            assert!(path.join("credentials.prepared").exists());
            assert!(!path.join("credentials").exists());
            std::fs::remove_dir(path.join("state")).unwrap();
        }
        let (author, capability) = profile
            .database_credentials(&database, root, 0, &key, &[])
            .unwrap();
        assert!(author.is_none());
        assert!(!capability.enrolled);
        assert_eq!(
            capability.transport_seed.as_slice(),
            transport.secret_seed().as_slice()
        );
        assert!(!path.join("credentials.prepared").exists());
        let (active, _) = profile.activate(&capability).unwrap();
        assert!(!active.permits_exchange());
        if state_exists {
            assert_eq!(
                active.load_state::<u64>("PUBLIC retained marker").unwrap(),
                Some(41)
            );
        }
    }
}

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
