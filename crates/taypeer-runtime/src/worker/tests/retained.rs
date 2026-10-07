//! PUBLIC scenarios retained from the removed CLI, through real worker dispatch and restart.
//! Local encrypted branches model late delivery; no network admission is implied.
use super::*;
use std::{
    io::Write,
    net::TcpListener,
    sync::atomic::{AtomicBool, AtomicUsize, Ordering},
    time::Duration,
};
use taypeer_core::{Color, EntryId, GroupId, LucideKey, OperationId};
use taypeer_services::{
    AttachmentEdit, BinaryEdit, BinaryRequest, BinaryTarget, BinaryView, EntryPatch, EntrySummary,
    EntryView, FieldUpdate, GroupMove, GroupSummary, IconInput, InspectionTarget, InspectionView,
    LifecycleAction, ObjectId, ObjectState, PendingSource, PreparedLifecycle, RecoveryMode,
    RecoveryRequest, RevisionSummary, SiblingPosition, StorageUsage, TreeView,
};
use taypeer_sync::{Backend, Command as SyncCommand, Reply};

fn request<T: serde::de::DeserializeOwned>(client: &Client, command: &Command) -> T {
    serde_json::from_value(client.request(command, false).unwrap()).unwrap()
}

fn stop(client: &Client) {
    client.control.invalidate(LockReason::Manual);
    assert_eq!(
        client.control.wait_closed().unwrap().termination,
        Termination::Graceful
    );
}

fn fixture(client: &Client) -> (GroupId, GroupId, EntryId) {
    let mut groups = Vec::new();
    for name in ["PUBLIC source group", "PUBLIC destination group"] {
        let group: GroupSummary = request(
            client,
            &Command::CreateGroup {
                operation: taypeer_services::new_operation_id().unwrap(),
                name: name.into(),
                parent: None,
            },
        );
        groups.push(group.id);
    }
    let entry = request(
        client,
        &Command::CreateEntry {
            group: groups[0].clone(),
            operation: taypeer_services::new_operation_id().unwrap(),
            patch: EntryPatch {
                title: FieldUpdate::Set("PUBLIC entry".into()),
                password: FieldUpdate::Set("PUBLIC retained secret".into()),
                ..Default::default()
            },
        },
    );
    (groups.remove(0), groups.remove(0), entry)
}

fn replay(client: &Client, path: &Path, command: &Command, expected: &Value) {
    let before = std::fs::read(path).unwrap();
    assert_eq!(request::<Value>(client, command), *expected);
    assert_eq!(std::fs::read(path).unwrap(), before);
}

#[test]
fn subtree_moves_trash_restore_and_purge_keep_exact_receipts_after_worker_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC lifecycle receipts.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let first = open(&sessions, &path, true);
    let (source, destination, entry) = fixture(&first);
    let move_entry = Command::MoveEntry {
        entry: entry.clone(),
        group: destination.clone(),
        review: None,
        operation: OperationId::new("PUBLIC move entry receipt"),
    };
    let moved: Value = request(&first, &move_entry);
    let clone = Command::CloneGroup {
        group: destination,
        parent: Some(source.clone()),
        name: Some("PUBLIC copied subtree".into()),
        operation: OperationId::new("PUBLIC clone subtree receipt"),
    };
    let cloned: Vec<ObjectId> = request(&first, &clone);
    assert_eq!(cloned.len(), 2);
    let group = cloned
        .iter()
        .find_map(|object| match object {
            ObjectId::Group(group) => Some(group.clone()),
            ObjectId::Entry(_) => None,
        })
        .unwrap();
    let copied_entry = cloned
        .iter()
        .find_map(|object| match object {
            ObjectId::Entry(entry) => Some(entry.clone()),
            ObjectId::Group(_) => None,
        })
        .unwrap();
    let move_group = Command::MoveGroup {
        request: GroupMove {
            group: group.clone(),
            parent: None,
            position: SiblingPosition::Before(source.clone()),
            review: None,
            name: None,
        },
        operation: OperationId::new("PUBLIC move group receipt"),
    };
    let moved_group: Value = request(&first, &move_group);
    let trash = Command::TrashObject {
        target: ObjectId::Group(group.clone()),
        operation: OperationId::new("PUBLIC trash subtree receipt"),
    };
    let trashed: Value = request(&first, &trash);
    stop(&first);

    let next = open(&sessions, &path, false);
    for (command, expected) in [
        (&move_entry, moved),
        (&clone, serde_json::to_value(&cloned).unwrap()),
        (&move_group, moved_group),
        (&trash, trashed),
    ] {
        replay(&next, &path, command, &expected);
    }
    let retained: Vec<ObjectState> = request(&next, &Command::Trash);
    assert_eq!(retained.len(), 2);
    let address = retained
        .iter()
        .find(|state| state.address.object == ObjectId::Entry(copied_entry.clone()))
        .unwrap()
        .address
        .clone();
    let inspected: InspectionView =
        request(&next, &Command::Inspect(InspectionTarget::Object(address)));
    assert!(
        !serde_json::to_string(&inspected)
            .unwrap()
            .contains("PUBLIC retained secret")
    );
    let prepared: PreparedLifecycle = request(
        &next,
        &Command::PrepareLifecycle {
            action: LifecycleAction::Restore,
            target: ObjectId::Group(group.clone()),
            destination: Some(source.clone()),
        },
    );
    let restore = Command::ConfirmLifecycle {
        prepared,
        operation: OperationId::new("PUBLIC restore subtree receipt"),
    };
    let restored: Value = request(&next, &restore);
    stop(&next);

    let next = open(&sessions, &path, false);
    replay(&next, &path, &restore, &restored);
    assert!(request::<Vec<ObjectState>>(&next, &Command::Trash).is_empty());
    let groups: Vec<GroupSummary> = request(&next, &Command::Groups);
    assert_eq!(
        groups.iter().find(|row| row.id == group).unwrap().parent,
        Some(source)
    );
    let history: Vec<RevisionSummary> = request(&next, &Command::History(copied_entry));
    assert_eq!(history.len(), 2);
    assert!(
        !serde_json::to_string(&history)
            .unwrap()
            .contains("PUBLIC retained secret")
    );
    request::<Value>(
        &next,
        &Command::TrashObject {
            target: ObjectId::Group(group.clone()),
            operation: OperationId::new("PUBLIC second trash receipt"),
        },
    );
    let prepared: PreparedLifecycle = request(
        &next,
        &Command::PrepareLifecycle {
            action: LifecycleAction::Purge,
            target: ObjectId::Group(group.clone()),
            destination: None,
        },
    );
    let purge = Command::ConfirmLifecycle {
        prepared,
        operation: OperationId::new("PUBLIC purge subtree receipt"),
    };
    let purged: Value = request(&next, &purge);
    stop(&next);

    let next = open(&sessions, &path, false);
    replay(&next, &path, &purge, &purged);
    let tree: TreeView = request(&next, &Command::Tree);
    assert!(
        tree.groups
            .iter()
            .all(|node| node.address.object != ObjectId::Group(group.clone()))
    );
    assert!(
        !serde_json::to_string(&tree)
            .unwrap()
            .contains("PUBLIC retained secret")
    );
    let entries: Vec<EntrySummary> = request(
        &next,
        &Command::Entries {
            group: None,
            query: String::new(),
        },
    );
    assert_eq!(entries.len(), 1);
    assert_eq!(entries[0].id, entry);
    stop(&next);
}

fn binary(target: BinaryTarget, edit: BinaryEdit, operation: &str) -> Command {
    Command::EditBinary {
        request: BinaryRequest {
            target,
            edit,
            review: None,
        },
        operation: OperationId::new(operation),
    }
}

#[test]
fn attachment_receipts_precede_missing_sources_and_historical_export_after_worker_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC binary receipts.taypeer");
    let input = directory.path().join("PUBLIC input.txt");
    let output = directory.path().join("PUBLIC historical output.txt");
    let sessions = SessionController::new(SessionPolicy::default());
    let first = open(&sessions, &path, true);
    let (group, _, entry) = fixture(&first);
    let target = BinaryTarget::Entry(entry.clone());
    std::fs::write(&input, b"PUBLIC original").unwrap();
    let add = binary(
        target.clone(),
        BinaryEdit::Attachment(AttachmentEdit::Add {
            path: input.clone(),
            name: None,
        }),
        "PUBLIC add attachment receipt",
    );
    let added: Value = request(&first, &add);
    let view: BinaryView = request(&first, &Command::BinaryView(target.clone()));
    assert_eq!(view.attachments.len(), 1);
    let attachment = view.attachments[0].id.clone();
    let blob = view.attachments[0].contents[0].id.clone();
    let history: Vec<RevisionSummary> = request(&first, &Command::History(entry.clone()));
    let revision = history.last().unwrap().id.clone();
    std::fs::remove_file(&input).unwrap();
    stop(&first);

    let next = open(&sessions, &path, false);
    replay(&next, &path, &add, &added);
    let rename = binary(
        target.clone(),
        BinaryEdit::Attachment(AttachmentEdit::Rename {
            attachment: attachment.clone(),
            name: "PUBLIC renamed attachment".into(),
        }),
        "PUBLIC rename attachment receipt",
    );
    request::<Value>(&next, &rename);
    std::fs::write(&input, b"PUBLIC replacement").unwrap();
    let replace = binary(
        target.clone(),
        BinaryEdit::Attachment(AttachmentEdit::Replace {
            attachment: attachment.clone(),
            path: input.clone(),
        }),
        "PUBLIC replace attachment receipt",
    );
    request::<Value>(&next, &replace);
    std::fs::remove_file(&input).unwrap();
    let mut confirmed = vec![add, rename, replace];
    for (scope, key, operation) in [
        (
            BinaryTarget::Group(group),
            "folder-open",
            "PUBLIC group icon receipt",
        ),
        (target.clone(), "key-round", "PUBLIC entry icon receipt"),
    ] {
        let command = binary(
            scope,
            BinaryEdit::Icon(IconInput::Lucide(
                LucideKey::try_from(key.to_owned()).unwrap(),
            )),
            operation,
        );
        request::<Value>(&next, &command);
        confirmed.push(command);
    }
    let appearance = binary(
        target.clone(),
        BinaryEdit::Appearance {
            foreground: FieldUpdate::Set(Color([18, 52, 86, 255])),
            background: FieldUpdate::Keep,
        },
        "PUBLIC appearance receipt",
    );
    request::<Value>(&next, &appearance);
    confirmed.push(appearance);
    let remove = binary(
        target.clone(),
        BinaryEdit::Attachment(AttachmentEdit::Remove { attachment }),
        "PUBLIC remove attachment receipt",
    );
    request::<Value>(&next, &remove);
    confirmed.push(remove);
    stop(&next);

    let next = open(&sessions, &path, false);
    for command in &confirmed {
        replay(&next, &path, command, &Value::Null);
    }
    let view: BinaryView = request(&next, &Command::BinaryView(target));
    assert!(view.attachments.is_empty());
    assert_eq!(
        serde_json::to_value(&view.icons).unwrap(),
        serde_json::json!([{"kind":"lucide", "value":"key-round"}])
    );
    assert_eq!(view.foreground, vec![Some(Color([18, 52, 86, 255]))]);
    let historical = BinaryTarget::Revision { entry, revision };
    let export = |path, overwrite| Command::ExportBinary {
        target: historical.clone(),
        blob: blob.clone(),
        path,
        overwrite,
    };
    let before = std::fs::read(&path).unwrap();
    request::<Value>(&next, &export(output.clone(), false));
    assert_eq!(std::fs::read(&output).unwrap(), b"PUBLIC original");
    assert!(next.request(&export(output, false), false).is_err());
    assert!(next.request(&export(path.clone(), true), false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let usage: StorageUsage = request(&next, &Command::StorageUsage);
    assert_eq!(
        usage.attachment_bytes,
        (b"PUBLIC original".len() + b"PUBLIC replacement".len()) as u64
    );
    let collect = Command::CollectBlobs(OperationId::new("PUBLIC collection receipt"));
    let collected: Value = request(&next, &collect);
    stop(&next);
    let next = open(&sessions, &path, false);
    replay(&next, &path, &collect, &collected);
    stop(&next);
}

struct FaviconServer {
    url: String,
    requests: Arc<AtomicUsize>,
    stop: Arc<AtomicBool>,
    thread: Option<std::thread::JoinHandle<()>>,
}

impl FaviconServer {
    fn start() -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        listener.set_nonblocking(true).unwrap();
        let url = format!("http://{}/page", listener.local_addr().unwrap());
        let requests = Arc::new(AtomicUsize::new(0));
        let stop = Arc::new(AtomicBool::new(false));
        let count = Arc::clone(&requests);
        let stopped = Arc::clone(&stop);
        let thread = std::thread::spawn(move || {
            while !stopped.load(Ordering::Relaxed) {
                match listener.accept() {
                    Ok((mut socket, _)) => {
                        socket
                            .set_read_timeout(Some(Duration::from_secs(30)))
                            .unwrap();
                        let mut request = Vec::new();
                        {
                            let mut reader = BufReader::new(&mut socket);
                            while request.len() < 8192 && !request.ends_with(b"\r\n\r\n") {
                                if reader.read_until(b'\n', &mut request).unwrap_or(0) == 0 {
                                    break;
                                }
                            }
                        }
                        if !request.ends_with(b"\r\n\r\n") {
                            continue;
                        }
                        count.fetch_add(1, Ordering::Relaxed);
                        let body = if request.starts_with(b"GET /page ") {
                            r#"<link rel="icon" href="/PUBLIC.svg">"#
                        } else {
                            r#"<svg xmlns="http://www.w3.org/2000/svg" width="24" height="24"><circle cx="12" cy="12" r="8"/></svg>"#
                        };
                        let _ = write!(
                            socket,
                            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                            body.len()
                        );
                    }
                    Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                        std::thread::sleep(Duration::from_millis(5));
                    }
                    Err(_) => break,
                }
            }
        });
        Self {
            url,
            requests,
            stop,
            thread: Some(thread),
        }
    }
}

impl Drop for FaviconServer {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        self.thread.take().unwrap().join().unwrap();
    }
}

#[test]
fn favicon_receipt_prevents_a_second_download_after_worker_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC favicon receipt.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let first = open(&sessions, &path, true);
    let (_, _, entry) = fixture(&first);
    let server = FaviconServer::start();
    let favicon = binary(
        BinaryTarget::Entry(entry.clone()),
        BinaryEdit::Icon(IconInput::Favicon(Some(server.url.clone()))),
        "PUBLIC favicon receipt",
    );
    let saved: Value = request(&first, &favicon);
    assert_eq!(server.requests.load(Ordering::Relaxed), 2);
    let before = std::fs::read(&path).unwrap();
    stop(&first);
    let next = open(&sessions, &path, false);
    replay(&next, &path, &favicon, &saved);
    assert_eq!(server.requests.load(Ordering::Relaxed), 2);
    let invalid = binary(
        BinaryTarget::Entry(entry),
        BinaryEdit::Icon(IconInput::Url("http://PUBLIC:PUBLIC@localhost".into())),
        "PUBLIC rejected credential URL",
    );
    assert!(next.request(&invalid, false).is_err());
    assert_eq!(server.requests.load(Ordering::Relaxed), 2);
    assert_eq!(std::fs::read(&path).unwrap(), before);
    stop(&next);
}

fn deliver_file(from: &Path, to: &Path) {
    let transport = Arc::new(TransportKey::from_seed(&[59; 32]));
    let coordinator = Coordinator::new(Arc::clone(&transport));
    coordinator
        .register(ArchiveStore::open(to, None, None).unwrap())
        .unwrap();
    let incoming = ArchiveSnapshot::open(from, None).unwrap();
    let reply = coordinator
        .command(
            transport.public(),
            SyncCommand::Offer(Box::new(incoming.metadata().clone())),
        )
        .unwrap();
    let Reply::Needed(needed) = reply else {
        panic!("expected PUBLIC local branch inventory");
    };
    assert!(!needed.is_empty());
    for id in needed {
        let mut staged = tempfile::NamedTempFile::new().unwrap();
        std::io::copy(&mut incoming.reader(id).unwrap(), &mut staged).unwrap();
        coordinator
            .receive(
                transport.public(),
                &incoming.chain().head().database,
                incoming.metadata().manifest.body.objects[&id].clone(),
                staged,
            )
            .unwrap();
    }
}

#[test]
fn late_source_recovery_requires_a_group_and_replays_without_an_extra_generation_after_restart() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC late source.taypeer");
    let branch = directory.path().join("PUBLIC late branch.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let first = open(&sessions, &path, true);
    let (_, destination, entry) = fixture(&first);
    stop(&first);
    std::fs::copy(&path, &branch).unwrap();
    let late = open(&sessions, &branch, false);
    request::<Value>(
        &late,
        &Command::UpdateEntry {
            id: entry.clone(),
            patch: EntryPatch {
                password: FieldUpdate::Set("PUBLIC late retained secret".into()),
                ..Default::default()
            },
            operation: OperationId::new("PUBLIC late entry change"),
        },
    );
    stop(&late);
    let next = open(&sessions, &path, false);
    request::<Value>(
        &next,
        &Command::TrashObject {
            target: ObjectId::Entry(entry.clone()),
            operation: OperationId::new("PUBLIC before late purge"),
        },
    );
    let prepared = request(
        &next,
        &Command::PrepareLifecycle {
            action: LifecycleAction::Purge,
            target: ObjectId::Entry(entry.clone()),
            destination: None,
        },
    );
    request::<Value>(
        &next,
        &Command::ConfirmLifecycle {
            prepared,
            operation: OperationId::new("PUBLIC late closed generation"),
        },
    );
    stop(&next);
    deliver_file(&branch, &path);

    let next = open(&sessions, &path, false);
    assert!(next.request(&Command::Entry(entry.clone()), false).is_err());
    assert!(
        next.request(&Command::RevealPassword(entry.clone()), false)
            .is_err()
    );
    let sources: Vec<PendingSource> = request(&next, &Command::PendingSources);
    assert_eq!(sources.len(), 1);
    let source = sources[0].id.clone();
    let inspection: InspectionView = request(
        &next,
        &Command::Inspect(InspectionTarget::Source(source.clone())),
    );
    assert!(
        !serde_json::to_string(&inspection)
            .unwrap()
            .contains("PUBLIC late retained secret")
    );
    let recover = |destination| Command::RecoverSource {
        request: Box::new(RecoveryRequest {
            icon: None,
            source: source.clone(),
            mode: RecoveryMode::Restore,
            destination,
            name: None,
            fields: None,
        }),
        operation: OperationId::new("PUBLIC recover late entry receipt"),
    };
    let before = std::fs::read(&path).unwrap();
    assert!(next.request(&recover(None), false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), before);
    let recovery = recover(Some(destination.clone()));
    let restored: Vec<ObjectId> = request(&next, &recovery);
    assert_eq!(restored, vec![ObjectId::Entry(entry.clone())]);
    let before = std::fs::read(&path).unwrap();
    stop(&next);

    let next = open(&sessions, &path, false);
    replay(
        &next,
        &path,
        &recovery,
        &serde_json::to_value(restored).unwrap(),
    );
    assert_eq!(std::fs::read(&path).unwrap(), before);
    assert!(request::<Vec<PendingSource>>(&next, &Command::PendingSources).is_empty());
    let shown: EntryView = request(&next, &Command::Entry(entry.clone()));
    assert_eq!(shown.group_id, destination);
    assert!(shown.has_password);
    assert!(
        !serde_json::to_string(&shown)
            .unwrap()
            .contains("PUBLIC late retained secret")
    );
    assert_eq!(
        request::<Value>(&next, &Command::RevealPassword(entry.clone())),
        "PUBLIC late retained secret"
    );
    assert_eq!(
        request::<Vec<RevisionSummary>>(&next, &Command::History(entry)).len(),
        1
    );
    stop(&next);
}
