//! PUBLIC encrypted files exercise shared selection, atomic streams and exact saved-entry replay.
use super::*;
use std::{fs, io, path::PathBuf};

const IMAGE: &[u8] = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#14dc28"/></svg>"##;
const PASSWORD: &[u8] = b"PUBLIC group favicon master";

struct Fixture {
    directory: tempfile::TempDir,
    path: PathBuf,
    service: DatabaseService,
    session: SessionToken,
    group: GroupId,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC favicons.taypeer");
        let mut service = DatabaseService::new();
        let session = service
            .create_file(&path, "PUBLIC batch".into(), PASSWORD)
            .unwrap();
        let group = service
            .create_group(
                &session,
                "PUBLIC group".into(),
                None,
                &new_operation_id().unwrap(),
            )
            .unwrap()
            .value
            .id;
        Self {
            directory,
            path,
            service,
            session,
            group,
        }
    }
    fn entry(&mut self, group: GroupId, title: &str, url: Option<&str>) -> EntryId {
        self.service
            .start_create_entry(&self.session, group)
            .unwrap();
        self.service
            .update_draft(
                &self.session,
                EditableEntry {
                    title: title.into(),
                    username: Some("PUBLIC unchanged username".into()),
                    url: url.map(str::to_owned),
                    ..Default::default()
                },
            )
            .unwrap();
        self.service
            .save_draft(&self.session, &new_operation_id().unwrap())
            .unwrap()
            .value
    }
    fn plan(&mut self, recursive: bool, replace: bool) -> GroupFaviconPlan {
        self.service
            .prepare_group_favicons(
                &self.session,
                GroupFaviconRequest {
                    group: self.group.clone(),
                    recursive,
                    replace,
                    operation: new_operation_id().unwrap(),
                },
            )
            .unwrap()
            .value
    }
    fn request(&mut self, entry: &EntryId) -> FaviconEntryRequest {
        self.plan(false, false)
            .rows
            .into_iter()
            .find_map(|row| match row.action {
                FaviconPlanAction::Acquire(request) if &request.entry == entry => Some(request),
                _ => None,
            })
            .unwrap()
    }
}

struct NeverRead;
impl Read for NeverRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        panic!("PUBLIC exact receipt or rejected intent must precede provider read")
    }
}
struct FailedRead;
impl Read for FailedRead {
    fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
}

#[test]
fn shared_selection_freezes_direct_recursive_replace_missing_url_and_parent_identity() {
    let mut fixture = Fixture::new();
    let child = fixture
        .service
        .create_group(
            &fixture.session,
            "PUBLIC child".into(),
            Some(fixture.group.clone()),
            &new_operation_id().unwrap(),
        )
        .unwrap()
        .value
        .id;
    let direct = fixture.entry(
        fixture.group.clone(),
        "PUBLIC direct",
        Some("https://public.invalid/direct"),
    );
    let missing = fixture.entry(fixture.group.clone(), "PUBLIC no URL", None);
    let custom = fixture.entry(
        fixture.group.clone(),
        "PUBLIC custom",
        Some("https://public.invalid/custom"),
    );
    let descendant = fixture.entry(
        child,
        "PUBLIC descendant",
        Some("https://public.invalid/descendant"),
    );
    fixture
        .service
        .edit_binary(
            &fixture.session,
            &BinaryRequest {
                target: BinaryTarget::Entry(custom.clone()),
                edit: BinaryEdit::Icon(IconInput::Lucide(
                    "key-round".to_owned().try_into().unwrap(),
                )),
                review: None,
            },
            &new_operation_id().unwrap(),
        )
        .unwrap();
    let before = fixture
        .service
        .history(&fixture.session, &direct)
        .unwrap()
        .value
        .len();
    let plan = fixture.plan(false, false);
    assert_eq!(plan.rows.len(), 3);
    assert!(
        plan.rows
            .iter()
            .any(|row| row.entry == missing && matches!(row.action, FaviconPlanAction::MissingUrl))
    );
    assert!(
        plan.rows
            .iter()
            .any(|row| row.entry == custom && matches!(row.action, FaviconPlanAction::Skipped))
    );
    assert!(!plan.rows.iter().any(|row| row.entry == descendant));
    assert_eq!(
        fixture
            .service
            .history(&fixture.session, &direct)
            .unwrap()
            .value
            .len(),
        before
    );
    let encrypted = fs::read(&fixture.path).unwrap();
    let mut changed = plan.request.clone();
    changed.recursive = true;
    assert_eq!(
        fixture
            .service
            .prepare_group_favicons(&fixture.session, changed)
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(fs::read(&fixture.path).unwrap(), encrypted);
    let expanded = fixture.plan(true, true);
    assert_eq!(expanded.rows.len(), 4);
    assert!(
        expanded
            .rows
            .iter()
            .any(|row| row.entry == custom && matches!(row.action, FaviconPlanAction::Acquire(_)))
    );
}

#[test]
fn saved_entry_apply_preserves_unfinished_form_and_exact_replay_cannot_rollback_later_fields() {
    let mut fixture = Fixture::new();
    let entry = fixture.entry(
        fixture.group.clone(),
        "PUBLIC saved",
        Some("https://public.invalid/original"),
    );
    let request = fixture.request(&entry);
    fixture
        .service
        .start_create_entry(&fixture.session, fixture.group.clone())
        .unwrap();
    fixture
        .service
        .patch_draft(
            &fixture.session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC unsaved sibling".into()),
                password: FieldUpdate::Set("PUBLIC unfinished secret".into()),
                ..Default::default()
            },
        )
        .unwrap();
    fixture.service.persist_drafts(&fixture.session).unwrap();
    let form = fixture
        .service
        .active_draft(&fixture.session)
        .unwrap()
        .value
        .unwrap();
    let sidecar = fixture
        .directory
        .path()
        .join("PUBLIC favicons.taypeer.draft");
    let local = fs::read(&sidecar).unwrap();
    fixture
        .service
        .import_entry_favicon_stream(&fixture.session, &request, IMAGE, IMAGE.len() as u64)
        .unwrap();
    assert_eq!(
        fixture
            .service
            .active_draft(&fixture.session)
            .unwrap()
            .value
            .unwrap(),
        form
    );
    assert_eq!(fs::read(&sidecar).unwrap(), local);
    assert_eq!(
        fixture
            .service
            .reveal_editor(&fixture.session, None)
            .unwrap()
            .expose(),
        "PUBLIC unfinished secret"
    );
    assert_eq!(
        fixture
            .service
            .history(&fixture.session, &entry)
            .unwrap()
            .value
            .len(),
        2
    );
    fixture.service.cancel_draft(&fixture.session).unwrap();
    fixture
        .service
        .start_edit_entry(&fixture.session, &entry)
        .unwrap();
    fixture
        .service
        .patch_draft(
            &fixture.session,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC later title".into()),
                url: FieldUpdate::Set("https://public.invalid/later".into()),
                ..Default::default()
            },
        )
        .unwrap();
    fixture
        .service
        .save_draft(&fixture.session, &new_operation_id().unwrap())
        .unwrap();
    let later = fs::read(&fixture.path).unwrap();
    assert!(
        fixture
            .service
            .favicon_entry_receipt(&fixture.session, &request)
            .unwrap()
            .value
    );
    fixture
        .service
        .import_entry_favicon_stream(&fixture.session, &request, NeverRead, u64::MAX)
        .unwrap();
    assert_eq!(fs::read(&fixture.path).unwrap(), later);
    assert_eq!(
        fixture
            .service
            .view_entry(&fixture.session, &entry)
            .unwrap()
            .value
            .title,
        "PUBLIC later title"
    );
    assert_eq!(
        fixture
            .service
            .history(&fixture.session, &entry)
            .unwrap()
            .value
            .len(),
        3
    );
    let mut changed = request.clone();
    changed.url = "https://public.invalid/different".into();
    assert_eq!(
        fixture
            .service
            .import_entry_favicon_stream(&fixture.session, &changed, NeverRead, 0)
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(fs::read(&fixture.path).unwrap(), later);
    fixture.service.lock(&fixture.session).unwrap();
    assert_eq!(
        fixture
            .service
            .favicon_entry_receipt(&fixture.session, &request)
            .err(),
        Some(ServiceError::Locked)
    );
}

#[test]
fn rejected_streams_preserve_exact_encrypted_file_history_and_blob_accounting() {
    let mut fixture = Fixture::new();
    let entry = fixture.entry(
        fixture.group.clone(),
        "PUBLIC atomic",
        Some("https://public.invalid/atomic"),
    );
    let request = fixture.request(&entry);
    let before = fs::read(&fixture.path).unwrap();
    let retained = fixture
        .service
        .storage_usage(&fixture.session)
        .unwrap()
        .value
        .retained_bytes;
    for (bytes, length, error) in [
        (IMAGE, IMAGE.len() as u64 - 1, ServiceError::InvalidInput),
        (IMAGE, IMAGE.len() as u64 + 1, ServiceError::InvalidInput),
        (
            b"PUBLIC invalid image".as_slice(),
            20,
            ServiceError::Icon(icons::IconError::InvalidImage),
        ),
        (
            IMAGE,
            ICON_LIMIT + 1,
            ServiceError::Icon(icons::IconError::TooLarge),
        ),
    ] {
        assert_eq!(
            fixture
                .service
                .import_entry_favicon_stream(&fixture.session, &request, bytes, length)
                .err(),
            Some(error)
        );
        assert_eq!(fs::read(&fixture.path).unwrap(), before);
        assert_eq!(
            fixture
                .service
                .history(&fixture.session, &entry)
                .unwrap()
                .value
                .len(),
            1
        );
        assert_eq!(
            fixture
                .service
                .storage_usage(&fixture.session)
                .unwrap()
                .value
                .retained_bytes,
            retained
        );
    }
    assert_eq!(
        fixture
            .service
            .import_entry_favicon_stream(&fixture.session, &request, FailedRead, IMAGE.len() as u64)
            .err(),
        Some(ServiceError::Icon(icons::IconError::File))
    );
    assert_eq!(fs::read(&fixture.path).unwrap(), before);
    let mut forged = request;
    forged.operation = new_operation_id().unwrap();
    assert_eq!(
        fixture
            .service
            .import_entry_favicon_stream(&fixture.session, &forged, NeverRead, 1)
            .err(),
        Some(ServiceError::InvalidInput)
    );
    assert_eq!(fs::read(&fixture.path).unwrap(), before);
}
