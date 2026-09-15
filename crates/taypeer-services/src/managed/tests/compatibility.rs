//! PUBLIC fixtures only; the profiles below never touch native credentials.
use super::*;
use taypeer_core::{ClientCapabilities, FeatureId, OperationId, SchemaDescriptor};

fn port(
    profile: &Profile,
    database: &DatabaseId,
    path: &std::path::Path,
) -> Box<dyn CipherPersistence> {
    Box::new(
        CoordinatorPersistence::new(
            Arc::clone(&profile.coordinator),
            database.clone(),
            path.into(),
            Digest::of(&profile.seed),
        )
        .unwrap(),
    )
}
fn read_only() -> ClientCapabilities {
    let schema = SchemaDescriptor::current();
    let write = schema
        .required_write_features()
        .iter()
        .filter(|id| id.as_str() != "taypeer.binary")
        .cloned()
        .collect();
    ClientCapabilities::default().restricted(schema.required_read_features(), &write)
}

#[test]
fn format_read_only_preserves_file_history_and_encrypted_draft() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(31);
    let path = directory.path().join("PUBLIC-read-only.taypeer");
    let (mut service, token) = create(&profile, path.clone());
    let group = service
        .create_group(&token, "PUBLIC group".into(), None)
        .unwrap()
        .value
        .id;
    service.start_create_entry(&token, group.clone()).unwrap();
    service
        .update_draft(
            &token,
            EditableEntry {
                title: "PUBLIC entry".into(),
                password: Some("PUBLIC password".into()),
                ..Default::default()
            },
        )
        .unwrap();
    let entry = service.save_draft(&token).unwrap().value;
    service.start_edit_entry(&token, &entry).unwrap();
    service
        .update_draft(
            &token,
            EditableEntry {
                title: "PUBLIC interrupted".into(),
                ..Default::default()
            },
        )
        .unwrap();
    service.lock(&token).unwrap();
    drop(service);
    let draft_path = directory.path().join(format!(
        "PUBLIC-read-only.taypeer.{}.draft",
        Digest::of(&profile.seed)
    ));
    let draft_bytes = std::fs::read(&draft_path).unwrap();
    let before = std::fs::read(&path).unwrap();
    let mut service = DatabaseService::with_capabilities(read_only());
    let session = service
        .open_managed(port(&profile, &token.database, &path), PASSWORD, || {
            Ok(Some(profile.author()))
        })
        .unwrap();
    let report = service.compatibility(&session).unwrap().value;
    assert!(report.read.is_supported());
    assert!(!report.write.is_supported());
    assert!(report.receive.is_supported());
    assert!(!service.can_write(&session).unwrap());
    assert_eq!(
        service
            .entries(&session, None, "PUBLIC entry")
            .unwrap()
            .value
            .len(),
        1
    );
    assert_eq!(service.history(&session, &entry).unwrap().value.len(), 1);
    assert!(service.reveal_password(&session, &entry).is_ok());
    assert_eq!(
        service
            .create_group(&session, "PUBLIC forbidden".into(), None)
            .unwrap_err(),
        ServiceError::WriteCompatibility
    );
    assert_eq!(
        service.start_edit_entry(&session, &entry).unwrap_err(),
        ServiceError::WriteCompatibility
    );
    assert_eq!(
        service.restore_draft(&session).unwrap_err(),
        ServiceError::WriteCompatibility
    );
    assert_eq!(
        service
            .collect_blobs(&session, &OperationId::new("PUBLIC gc"))
            .unwrap_err(),
        ServiceError::WriteCompatibility
    );
    assert_eq!(
        service.collect_received(&session).unwrap_err(),
        ServiceError::WriteCompatibility
    );
    assert_eq!(
        service.apply_received(&session).unwrap_err(),
        ServiceError::WriteCompatibility
    );
    assert_eq!(
        service.rotate_password(
            &session,
            Digest::of(b"PUBLIC forbidden rotation"),
            NEW_PASSWORD,
            None
        ),
        Err(ServiceError::WriteCompatibility)
    );
    assert!(matches!(
        service.prepare_trust_recovery(
            &session,
            NEW_PASSWORD,
            profile.identity(),
            Digest::of(b"PUBLIC forbidden recovery")
        ),
        Err(ServiceError::WriteCompatibility)
    ));
    service.lock(&session).unwrap();
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert_eq!(std::fs::read(&draft_path).unwrap(), draft_bytes);
    drop(service);
    let mut updated = DatabaseService::new();
    let session = updated
        .open_managed(port(&profile, &token.database, &path), PASSWORD, || {
            Ok(Some(profile.author()))
        })
        .unwrap();
    assert!(updated.can_write(&session).unwrap());
    assert_eq!(
        updated.restore_draft(&session).unwrap().value.fields.title,
        "PUBLIC interrupted"
    );
}

#[test]
fn ciphertext_waits_for_a_capable_client_without_being_received_twice() {
    let directory = tempfile::tempdir().unwrap();
    let a = Profile::new(32);
    let b = Profile::new(33);
    let path = directory.path().join("PUBLIC receiver.taypeer");
    let (mut sender, token) = create(&a, directory.path().join("PUBLIC sender.taypeer"));
    admit(&mut sender, &token, &a, &b);
    copy_to(&a, &token.database, &path);
    let (mut receiver, receiver_token) = b.open(path.clone(), PASSWORD);
    receiver.lock(&receiver_token).unwrap();
    drop(receiver);
    sender
        .create_group(&token, "PUBLIC incoming".into(), None)
        .unwrap();
    let before = std::fs::read(&path).unwrap();
    deliver(&a, &b, &token.database);
    assert_ne!(std::fs::read(&path).unwrap(), before);
    let received = std::fs::read(&path).unwrap();
    let schema = SchemaDescriptor::current();
    let caps = ClientCapabilities::default()
        .restricted(&BTreeSet::new(), schema.required_write_features());
    let mut blind = DatabaseService::with_capabilities(caps);
    assert_eq!(
        blind.open_managed(port(&b, &token.database, &path), PASSWORD, || panic!(
            "must not acquire author credentials for an unsupported reader"
        )),
        Err(ServiceError::ReadCompatibility)
    );
    assert!(blind.databases().is_empty());
    assert_eq!(std::fs::read(&path).unwrap(), received);
    let mut reader = DatabaseService::with_capabilities(read_only());
    let session = reader
        .open_managed(port(&b, &token.database, &path), PASSWORD, || {
            Ok(Some(b.author()))
        })
        .unwrap();
    assert_eq!(
        reader.apply_received(&session).unwrap_err(),
        ServiceError::WriteCompatibility
    );
    assert!(reader.groups(&session).unwrap().value.is_empty());
    reader.lock(&session).unwrap();
    drop(reader);
    assert_eq!(std::fs::read(&path).unwrap(), received);
    let mut updated = DatabaseService::new();
    let session = updated
        .open_managed(port(&b, &token.database, &path), PASSWORD, || {
            Ok(Some(b.author()))
        })
        .unwrap();
    updated.apply_received(&session).unwrap();
    assert_eq!(updated.groups(&session).unwrap().value.len(), 1);
    assert_eq!(updated.apply_received(&session).unwrap().value.applied, 0);
    updated.lock(&session).unwrap();
    drop(updated);
    b.coordinator.unregister(&token.database).unwrap();
    let (updated, session) = b.open(path, PASSWORD);
    assert_eq!(
        updated.groups(&session).unwrap().value[0].name,
        "PUBLIC incoming"
    );
}

#[test]
fn signed_descriptor_must_match_the_decrypted_document() {
    let directory = tempfile::tempdir().unwrap();
    let profile = Profile::new(34);
    let path = directory.path().join("PUBLIC mismatch.taypeer");
    let (mut service, session) = create(&profile, path.clone());
    let state = service.databases.get(&session.database).unwrap();
    let managed = state.managed.as_ref().unwrap();
    let reads = SchemaDescriptor::current().required_read_features().clone();
    let mut writes = reads.clone();
    writes.insert(FeatureId::new("future.retention").unwrap());
    let descriptor = SchemaDescriptor::new(5, reads, writes).unwrap();
    let chain = ControlChain::genesis(
        session.database.clone(),
        profile.identity(),
        &profile.author(),
        managed
            .open
            .as_ref()
            .unwrap()
            .metadata
            .commitment()
            .unwrap(),
        descriptor,
    )
    .unwrap();
    assert_eq!(
        managed.open.as_ref().unwrap().metadata.verify(
            state.document(),
            &chain,
            chain.head_hash().unwrap()
        ),
        Err(ServiceError::InvalidDocument)
    );
    service.lock(&session).unwrap();
    let before = std::fs::read(&path).unwrap();
    let port = port(&profile, &session.database, &path);
    let mut fresh = DatabaseService::new();
    assert!(matches!(
        fresh.open_managed(port, b"PUBLIC wrong password", || panic!(
            "credentials must wait for authentication"
        )),
        Err(ServiceError::Storage(StorageError::Authentication))
    ));
    assert_eq!(std::fs::read(path).unwrap(), before);
}
