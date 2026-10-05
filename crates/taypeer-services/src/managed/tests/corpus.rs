//! Frozen PUBLIC development files. Ordinary tests never invoke the writer to build inputs.
use super::*;
use std::path::Path;
use taypeer_core::{ClientCapabilities, OperationId};

const CORPUS_PASSWORD: &[u8] = b"PUBLIC_SESSION_DRAFT_PASSWORD";
const CASES: [&str; 3] = ["empty", "populated", "conflicts"];
fn corpus() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../../tests/fixtures/dev6")
}
fn draft_path(path: &Path) -> PathBuf {
    path.with_file_name(format!(
        "{}.{}.draft",
        path.file_name().unwrap().to_str().unwrap(),
        Digest::of(b"PUBLIC working copy")
    ))
}
fn open_case(
    profile: &Profile,
    path: &Path,
    capabilities: ClientCapabilities,
) -> (DatabaseService, SessionToken) {
    let store = ArchiveStore::open(path, None, None).unwrap();
    let database = store.snapshot().chain().head().database.clone();
    profile.coordinator.register(store).unwrap();
    let port = CoordinatorPersistence::new(
        Arc::clone(&profile.coordinator),
        database,
        path.into(),
        Digest::of(b"PUBLIC working copy"),
    )
    .unwrap();
    let mut service = DatabaseService::with_capabilities(capabilities);
    let token = service
        .open_managed(Box::new(port), CORPUS_PASSWORD, || {
            Ok(Some(profile.author()))
        })
        .unwrap();
    (service, token)
}
fn create_case(profile: &Profile, path: &Path) -> (DatabaseService, SessionToken) {
    let seed = DatabaseService::prepare_managed(
        "PUBLIC compatibility corpus".into(),
        CORPUS_PASSWORD,
        &profile.author(),
        profile.identity(),
        1,
        DatabasePolicy::new(1024 * 1024, 2 * 1024 * 1024, 500).unwrap(),
    )
    .unwrap();
    drop(seed.create(path, &profile.transport, None).unwrap());
    open_case(profile, path, ClientCapabilities::default())
}
fn snapshot(service: &DatabaseService, token: &SessionToken) -> serde_json::Value {
    let document = service.checked(token).unwrap().document();
    let entries = document.entries().unwrap();
    let histories: Vec<_> = entries
        .iter()
        .map(|entry| document.history(&entry.id).unwrap())
        .collect();
    let sources: Vec<_> = document
        .changes_since(&[])
        .unwrap()
        .iter()
        .map(|change| change.metadata().clone())
        .collect();
    serde_json::json!({
        "database": token.database, "schema": document.schema_descriptor().unwrap(),
        "groups": document.tree().unwrap(), "objects": document.object_states().unwrap(),
        "entries": entries, "histories": histories, "sources": sources,
        "pending": document.pending_sources().unwrap(),
        "received": service.received_sources(token).unwrap().value,
    })
}
fn add_entry(
    service: &mut DatabaseService,
    token: &SessionToken,
    group: &GroupId,
    title: &str,
) -> EntryId {
    service.start_create_entry(token, group.clone()).unwrap();
    service
        .update_draft(
            token,
            EditableEntry {
                title: title.into(),
                username: Some("PUBLIC user".into()),
                password: Some("PUBLIC saved password".into()),
                attributes: vec![EditableAttribute {
                    id: None,
                    name: "PUBLIC protected".into(),
                    value: "PUBLIC protected value".into(),
                    protected: true,
                }],
                ..Default::default()
            },
        )
        .unwrap();
    service
        .save_draft(token, &crate::new_operation_id().unwrap())
        .unwrap()
        .value
}
fn lifecycle(
    service: &mut DatabaseService,
    token: &SessionToken,
    entry: &EntryId,
    action: LifecycleAction,
    id: &str,
) {
    let prepared = service
        .prepare_lifecycle(token, action, ObjectId::Entry(entry.clone()), None)
        .unwrap()
        .value;
    service
        .confirm_lifecycle(token, &prepared, &OperationId::new(id))
        .unwrap();
}

/// Run only explicitly into a new output directory; frozen inputs are never overwritten.
#[test]
#[ignore = "explicit PUBLIC corpus generator; requires TAYPEER_CORPUS_OUTPUT"]
fn generate_development_corpus() {
    let output = PathBuf::from(
        std::env::var_os("TAYPEER_CORPUS_OUTPUT").expect("explicit output directory"),
    );
    std::fs::create_dir(&output).unwrap();
    for case in CASES {
        let temp = tempfile::tempdir().unwrap();
        let path = temp.path().join(format!("{case}.taypeer"));
        let profile = Profile::new(19);
        let (mut service, token) = create_case(&profile, &path);
        if case != "empty" {
            let group = service
                .create_group(
                    &token,
                    "PUBLIC group".into(),
                    None,
                    &crate::new_operation_id().unwrap(),
                )
                .unwrap()
                .value
                .id;
            let entry = add_entry(&mut service, &token, &group, "PUBLIC entry");
            let input = temp.path().join("PUBLIC attachment");
            std::fs::write(&input, b"PUBLIC corpus attachment bytes").unwrap();
            service
                .edit_binary(
                    &token,
                    &crate::BinaryRequest {
                        target: crate::BinaryTarget::Entry(entry.clone()),
                        review: None,
                        edit: crate::BinaryEdit::Attachment(crate::AttachmentEdit::Add {
                            path: input,
                            name: Some("PUBLIC attachment".into()),
                        }),
                    },
                    &OperationId::new("PUBLIC corpus attachment"),
                )
                .unwrap();
            let trashed = add_entry(&mut service, &token, &group, "PUBLIC trash");
            lifecycle(
                &mut service,
                &token,
                &trashed,
                LifecycleAction::Trash,
                "PUBLIC trash receipt",
            );
            let purged = add_entry(&mut service, &token, &group, "PUBLIC purge");
            lifecycle(
                &mut service,
                &token,
                &purged,
                LifecycleAction::Trash,
                "PUBLIC before purge receipt",
            );
            lifecycle(
                &mut service,
                &token,
                &purged,
                LifecycleAction::Purge,
                "PUBLIC purge receipt",
            );
            if case == "conflicts" {
                let peer = Profile::new(20);
                admit(&mut service, &token, &profile, &peer);
                let peer_path = temp.path().join("peer.taypeer");
                copy_to(&profile, &token.database, &peer_path);
                let (mut other, other_token) =
                    open_case(&peer, &peer_path, ClientCapabilities::default());
                for (db, session, title) in [
                    (&mut service, &token, "PUBLIC left"),
                    (&mut other, &other_token, "PUBLIC right"),
                ] {
                    db.start_edit_entry(session, &entry).unwrap();
                    db.patch_draft(
                        session,
                        EntryPatch {
                            title: FieldUpdate::Set(title.into()),
                            ..Default::default()
                        },
                    )
                    .unwrap();
                    db.save_draft(session, &crate::new_operation_id().unwrap())
                        .unwrap();
                }
                deliver(&peer, &profile, &token.database);
                service.apply_received(&token).unwrap();
                assert!(
                    !service
                        .conflicts(&token, &entry)
                        .unwrap()
                        .value
                        .fields
                        .is_empty()
                );
                other
                    .create_group(
                        &other_token,
                        "PUBLIC pending source".into(),
                        None,
                        &crate::new_operation_id().unwrap(),
                    )
                    .unwrap();
                deliver(&peer, &profile, &token.database);
                // Receipt is intentionally not followed by application.
                assert!(!service.received_sources(&token).unwrap().value.is_empty());
                other.lock(&other_token).unwrap();
            } else {
                service.start_edit_entry(&token, &entry).unwrap();
                service
                    .patch_draft(
                        &token,
                        EntryPatch {
                            title: FieldUpdate::Set("PUBLIC deferred corpus draft".into()),
                            ..Default::default()
                        },
                    )
                    .unwrap();
            }
        }
        let expected = snapshot(&service, &token);
        service.lock(&token).unwrap();
        std::fs::copy(&path, output.join(format!("{case}.taypeer"))).unwrap();
        if draft_path(&path).exists() {
            std::fs::copy(draft_path(&path), output.join(format!("{case}.draft"))).unwrap();
        }
        std::fs::write(
            output.join(format!("{case}.json")),
            serde_json::to_vec_pretty(&expected).unwrap(),
        )
        .unwrap();
    }
    let mut sums = String::new();
    let mut files: Vec<_> = std::fs::read_dir(&output)
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    files.sort();
    for path in files {
        sums.push_str(&format!(
            "{}  {}\n",
            Digest::of(&std::fs::read(&path).unwrap()),
            path.file_name().unwrap().to_str().unwrap()
        ));
    }
    std::fs::write(output.join("SHA256SUMS"), sums).unwrap();
}

#[test]
fn frozen_development_files_preserve_domain_state_and_original_sources() {
    for case in CASES {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(format!("{case}.taypeer"));
        let original = std::fs::read(corpus().join(format!("{case}.taypeer"))).unwrap();
        std::fs::write(&path, &original).unwrap();
        let expected: serde_json::Value =
            serde_json::from_slice(&std::fs::read(corpus().join(format!("{case}.json"))).unwrap())
                .unwrap();
        let profile = Profile::new(19);
        let (mut service, token) = open_case(&profile, &path, ClientCapabilities::default());
        assert_eq!(
            snapshot(&service, &token),
            expected,
            "PUBLIC fixture: {case}"
        );
        service.lock(&token).unwrap();
        assert_eq!(std::fs::read(&path).unwrap(), original);
    }
}

#[test]
fn frozen_corpus_checksums_do_not_depend_on_the_current_writer() {
    for line in std::fs::read_to_string(corpus().join("SHA256SUMS"))
        .unwrap()
        .lines()
    {
        let (digest, file) = line.split_once("  ").unwrap();
        assert_eq!(
            Digest::of(&std::fs::read(corpus().join(file)).unwrap()).to_string(),
            digest,
            "PUBLIC fixture: {file}"
        );
    }
}

#[test]
fn frozen_local_collection_automatically_resumes_exact_input_after_reopen() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC collection.taypeer");
    std::fs::copy(corpus().join("populated.taypeer"), &path).unwrap();
    std::fs::copy(corpus().join("populated.draft"), draft_path(&path)).unwrap();
    let original = std::fs::read(draft_path(&path)).unwrap();
    let profile = Profile::new(19);
    let (mut service, token) = open_case(&profile, &path, ClientCapabilities::default());
    assert_eq!(std::fs::read(draft_path(&path)).unwrap(), original);
    let drafts = service.drafts(&token).unwrap().value;
    assert_eq!(drafts.len(), 1);
    let entry = match &drafts[0].identity.target {
        crate::DraftTarget::Entry(entry) => entry.clone(),
        _ => panic!("PUBLIC corpus must contain an existing entry form"),
    };
    service.start_edit_entry(&token, &entry).unwrap();
    let view = service.editor_view(&token).unwrap();
    assert_eq!(view.identity.draft, drafts[0].identity.draft);
    assert_eq!(view.fields.title, "PUBLIC deferred corpus draft");
    assert!(view.fields.password.is_none());
    assert!(view.dirty);
}

#[test]
fn frozen_blobs_and_waiting_sources_survive_collection_and_reopen() {
    for case in ["populated", "conflicts"] {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join(format!("{case}.taypeer"));
        std::fs::copy(corpus().join(format!("{case}.taypeer")), &path).unwrap();
        let profile = Profile::new(19);
        let (mut service, token) = open_case(&profile, &path, ClientCapabilities::default());
        let before = snapshot(&service, &token);
        let entry = service.entries(&token, None, "").unwrap().value[0]
            .id
            .clone();
        let target = crate::BinaryTarget::Entry(entry);
        let binary = service.binary_view(&token, &target).unwrap().value;
        assert_eq!(binary.attachments.len(), 1);
        let blob = binary.attachments[0].contents[0].id.clone();
        service.collect_received(&token).unwrap();
        service.lock(&token).unwrap();
        drop(service);
        profile.coordinator.unregister(&token.database).unwrap();
        let (mut service, token) = open_case(&profile, &path, ClientCapabilities::default());
        assert_eq!(snapshot(&service, &token), before);
        let export = directory.path().join("PUBLIC exported attachment");
        service
            .export_binary(&token, &target, &blob, &export, false)
            .unwrap();
        assert_eq!(
            std::fs::read(export).unwrap(),
            b"PUBLIC corpus attachment bytes"
        );
        service.lock(&token).unwrap();
    }
}
