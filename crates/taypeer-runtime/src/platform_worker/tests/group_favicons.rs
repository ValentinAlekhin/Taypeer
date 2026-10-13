//! Actual isolated PUBLIC worker imports a selected favicon without HTTP or original bytes in JSON.
use super::*;
use taypeer_services::{FaviconPlanAction, GroupFaviconPlan, GroupFaviconRequest};

#[test]
fn group_favicon_selected_gateway_replays_before_opening_input_and_keeps_local_editor() {
    let fixture = Fixture::new();
    let mut bytes = br##"<svg xmlns="http://www.w3.org/2000/svg" width="16" height="16"><rect width="16" height="16" fill="#14dc28"/><!--"##.to_vec();
    bytes.extend(std::iter::repeat_n(b'P', 150_000));
    bytes.extend_from_slice(b"--></svg>");
    fixture
        .launcher
        .faults
        .selected_inputs
        .lock()
        .unwrap()
        .insert(1, io::Cursor::new(bytes.clone()));
    let mut worker = fixture.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC group stream entry");
    worker
        .request(&Command::PatchDraft(EntryPatch {
            url: FieldUpdate::Set("https://public.invalid/frozen".into()),
            ..Default::default()
        }))
        .unwrap();
    let initial = editor(&mut worker);
    save(
        &mut worker,
        &initial,
        taypeer_services::new_operation_id().unwrap(),
    )
    .unwrap();
    let entry = editor(&mut worker).entry.unwrap();
    let group = initial.group;
    let plan: GroupFaviconPlan = serde_json::from_value(
        worker
            .request(&Command::PrepareGroupFavicons(GroupFaviconRequest {
                group,
                recursive: false,
                replace: false,
                operation: taypeer_services::new_operation_id().unwrap(),
            }))
            .unwrap(),
    )
    .unwrap();
    let request = match &plan.rows[0].action {
        FaviconPlanAction::Acquire(request) => request.clone(),
        _ => panic!("PUBLIC favicon row must be selected"),
    };
    assert_eq!(request.entry, entry);
    edit(&mut worker, "PUBLIC unfinished editor remains");
    let unfinished = editor(&mut worker);
    worker
        .request(&Command::ImportSelectedEntryFavicon {
            request: request.clone(),
            input: 1,
            length: bytes.len() as u64,
        })
        .unwrap();
    assert_eq!(editor(&mut worker).identity, unfinished.identity);
    let history = worker.request(&Command::History(entry.clone())).unwrap();
    assert_eq!(history.as_array().unwrap().len(), 2);
    let reads = fixture.launcher.faults.input_reads.load(Ordering::Acquire);
    assert!(reads > 1);
    assert!(
        fixture
            .launcher
            .faults
            .largest_input_read
            .load(Ordering::Acquire)
            <= 65536
    );
    // Missing capability 999 would fail in the real private gateway if touched.
    worker
        .request(&Command::ImportSelectedEntryFavicon {
            request: request.clone(),
            input: 999,
            length: u64::MAX,
        })
        .unwrap();
    assert_eq!(
        fixture.launcher.faults.input_reads.load(Ordering::Acquire),
        reads
    );
    let mut changed = request.clone();
    changed.url = "https://public.invalid/changed".into();
    assert!(matches!(
        worker.request(&Command::ImportSelectedEntryFavicon {
            request: changed,
            input: 999,
            length: 1
        }),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::InvalidInput
        ))
    ));
    assert_eq!(
        fixture.launcher.faults.input_reads.load(Ordering::Acquire),
        reads
    );
    assert_eq!(editor(&mut worker).identity, unfinished.identity);
    worker.close().unwrap();
}
