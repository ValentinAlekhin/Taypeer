//! Public credentials exercise enrollment binding and historical-key rewrapping without sockets.
use super::*;
use taypeer_services::DatabaseService;
use taypeer_sync::{Backend, Coordinator, CoordinatorPersistence, EndpointAddr};
use taypeer_trust::{InvitationSecret, JoinProof};

#[test]
fn directory_symlink_escape_does_not_create_or_chmod_outside_the_profile() {
    use std::os::unix::fs::{PermissionsExt, symlink};
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().join("PUBLIC profile");
    let outside = directory.path().join("PUBLIC preserved");
    std::fs::create_dir(&profile).unwrap();
    std::fs::create_dir(&outside).unwrap();
    std::fs::set_permissions(&outside, std::fs::Permissions::from_mode(0o750)).unwrap();
    symlink(&outside, profile.join("escape")).unwrap();
    for path in [profile.join("escape"), profile.join("escape/new/leaf")] {
        assert!(matches!(
            private_directory(&path, &profile),
            Err(ProfileError::Invalid)
        ));
    }
    assert!(!outside.join("new").exists());
    assert_eq!(
        std::fs::metadata(outside).unwrap().permissions().mode() & 0o777,
        0o750
    );
}

#[test]
fn bounded_profile_hierarchy_confirms_ancestry_up_to_its_provisioned_base() {
    let directory = tempfile::tempdir().unwrap();
    let base = directory.path().canonicalize().unwrap();
    let profile = directory
        .path()
        .join("PUBLIC application")
        .join("profiles")
        .join("PUBLIC profile");
    let mut confirmed = Vec::new();
    private_directory_with_sync(&profile, &base, |path| {
        confirmed.push(path.to_owned());
        Ok(())
    })
    .unwrap();
    let profile = profile.canonicalize().unwrap();
    assert_eq!(
        confirmed,
        profile
            .ancestors()
            .take_while(|path| path.starts_with(&base))
            .map(Path::to_owned)
            .collect::<Vec<_>>()
    );
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(profile).unwrap().permissions().mode() & 0o777,
        0o700
    );
}

#[test]
fn directory_retry_confirms_existing_names_and_stops_at_the_profile_boundary() {
    let directory = tempfile::tempdir().unwrap();
    let profile = directory.path().canonicalize().unwrap();
    // Both credential and invitation paths create more than one new directory.
    for relative in [
        "databases/PUBLIC database/PUBLIC root",
        "joins/PUBLIC request",
    ] {
        let leaf = profile.join(relative);
        let rejected = leaf.parent().unwrap();
        let mut attempted = Vec::new();
        assert!(matches!(
            private_directory_with_sync(&leaf, &profile, |path| {
                attempted.push(path.to_owned());
                if path == rejected {
                    Err(std::io::Error::other("PUBLIC directory barrier denied"))
                } else {
                    Ok(())
                }
            }),
            Err(ProfileError::CommitUncertain)
        ));
        assert!(leaf.is_dir());
        assert_eq!(attempted, [leaf.clone(), rejected.to_owned()]);

        let mut confirmed = Vec::new();
        private_directory_with_sync(&leaf, &profile, |path| {
            assert!(path.starts_with(&profile));
            confirmed.push(path.to_owned());
            Ok(())
        })
        .unwrap();
        let expected: Vec<_> = leaf
            .ancestors()
            .take_while(|path| path.starts_with(&profile))
            .map(Path::to_owned)
            .collect();
        assert_eq!(confirmed, expected);
        assert_eq!(confirmed.last(), Some(&profile));
    }
}

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
        private_directory(&path, profile.directory()).unwrap();
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
