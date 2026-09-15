//! PUBLIC-only process fixture: real dispatch, service and dev5 files, no native credentials.
mod stalled;
use super::*;
use crate::{
    process::{CallbackHandler, Client, PipeActor},
    session::{
        DraftDisposition, LockReason, ProcessControl, SessionController, SessionPolicy, Termination,
    },
};
use std::{
    io::{BufRead, BufReader},
    path::Path,
    process::{Command as Process, Stdio},
    sync::mpsc,
};
use taypeer_storage::{ArchiveSnapshot, ArchiveStore};
use taypeer_sync::{Coordinator, CoordinatorPersistence};
use taypeer_trust::{AuthorKey, Digest, Identity, TransportKey};

const PASSWORD: &str = "PUBLIC_SESSION_DRAFT_PASSWORD";
const MARKER: &str = "PUBLIC IPC starts here";

fn synthetic_open(
    boot: &mut Boot,
    _: &Arc<Mutex<Channel>>,
    service: &mut DatabaseService,
) -> Result<SessionToken, RuntimeError> {
    let author = || AuthorKey::from_seed(&[19; 32]);
    let transport = Arc::new(TransportKey::from_seed(&[59; 32]));
    if let Some(name) = boot.create_name.take() {
        let seed = DatabaseService::prepare_managed(
            name,
            boot.password.as_bytes(),
            &author(),
            Identity::new(author().public(), transport.public()).unwrap(),
            1,
            Default::default(),
        )?;
        drop(
            seed.create(&boot.path, &transport, None)
                .map_err(crate::cipher_ipc::storage)?,
        );
    }
    let snapshot = ArchiveSnapshot::open(&boot.path, None).map_err(crate::cipher_ipc::storage)?;
    let database = snapshot.chain().head().database.clone();
    let store = ArchiveStore::open(&boot.path, Some(snapshot.chain().root().unwrap()), None)
        .map_err(crate::cipher_ipc::storage)?;
    let coordinator = Arc::new(Coordinator::new(transport));
    coordinator
        .register(store)
        .map_err(crate::host::sync_error)?;
    let port = CoordinatorPersistence::new(
        coordinator,
        database,
        boot.path.clone(),
        Digest::of(b"PUBLIC working copy"),
    )
    .map_err(crate::host::sync_error)?;
    Ok(service.open_managed(
        Box::new(stalled::Persistence(port)),
        boot.password.as_bytes(),
        || Ok(Some(author())),
    )?)
}

#[test]
#[ignore = "PUBLIC subprocess helper; invoked by the draft lifecycle tests"]
fn synthetic_child() {
    if std::env::var_os("TAYPEER_PUBLIC_DRAFT_TEST").is_none() {
        return;
    }
    // Skip the test harness prefix before switching stdout to private framed IPC.
    println!("\n{MARKER}");
    std::io::stdout().flush().unwrap();
    let result = run_with_open(std::io::stdin(), std::io::stdout(), synthetic_open);
    std::process::exit(if result.is_ok() { 0 } else { 70 });
}

struct NoIo;
impl CallbackHandler for NoIo {
    fn handle(
        &mut self,
        _: crate::cipher_ipc::IoRequest,
    ) -> Result<crate::cipher_ipc::IoValue, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
}
fn open(sessions: &SessionController, path: &Path, create: bool) -> Arc<Client> {
    let mut child = Process::new(std::env::current_exe().unwrap())
        .args([
            "--exact",
            "worker::tests::synthetic_child",
            "--ignored",
            "--nocapture",
        ])
        .env("TAYPEER_PUBLIC_DRAFT_TEST", "1")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let mut output = BufReader::new(child.stdout.take().unwrap());
    let (jobs, receive) = mpsc::sync_channel(1);
    let control = Arc::new(ProcessControl::new(
        child,
        jobs.clone(),
        sessions.activity(),
    ));
    control.set_generation(sessions.register(&control).unwrap());
    loop {
        let mut line = String::new();
        assert_ne!(output.read_line(&mut line).unwrap(), 0);
        if line.trim() == MARKER {
            break;
        }
    }
    let spool = tempfile::tempdir().unwrap();
    let boot = Boot {
        path: path.to_owned(),
        password: PASSWORD.into(),
        create_name: create.then(|| "PUBLIC database".into()),
        profile: spool.path().to_owned(),
        spool: spool.path().to_owned(),
        invitation: None,
    };
    let actor = PipeActor {
        input,
        output,
        callbacks: Box::new(NoIo),
        spool,
    };
    let running = Arc::clone(&control);
    std::thread::spawn(move || actor.run(receive, running));
    let client = Arc::new(Client { control, jobs });
    let database = client.request(&boot, true).unwrap();
    client
        .control
        .opened(serde_json::from_value(database).unwrap())
        .unwrap();
    client
}
fn edit(client: &Client) {
    let group = client
        .request(
            &Command::CreateGroup {
                name: "PUBLIC group".into(),
                parent: None,
            },
            false,
        )
        .unwrap();
    client
        .request(
            &Command::BeginCreate(serde_json::from_value(group["id"].clone()).unwrap()),
            false,
        )
        .unwrap();
    client
        .request(
            &Command::PatchDraft(taypeer_services::EntryPatch {
                title: taypeer_services::FieldUpdate::Set("PUBLIC unsaved title".into()),
                password: taypeer_services::FieldUpdate::Set("PUBLIC draft secret".into()),
                ..Default::default()
            }),
            false,
        )
        .unwrap();
}
fn draft_path(path: &Path) -> std::path::PathBuf {
    path.with_file_name(format!(
        "{}.{working_copy}.draft",
        path.file_name().unwrap().to_str().unwrap(),
        working_copy = Digest::of(b"PUBLIC working copy")
    ))
}

#[test]
fn lock_durably_preserves_an_encrypted_draft_for_a_new_process() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let first = open(&sessions, &path, true);
    edit(&first);
    let archive = std::fs::read(&path).unwrap();
    sessions.lock_all(LockReason::Idle);
    let outcome = first.control.wait_closed().unwrap();
    assert_eq!(outcome.draft, DraftDisposition::Preserved);
    assert_eq!(outcome.termination, Termination::Graceful);
    assert_eq!(std::fs::read(&path).unwrap(), archive);
    let draft = std::fs::read(draft_path(&path)).unwrap();
    assert!(
        !draft
            .windows(b"PUBLIC draft secret".len())
            .any(|w| w == b"PUBLIC draft secret")
    );
    let next = open(&sessions, &path, false);
    assert_ne!(
        first.control.status().generation,
        next.control.status().generation
    );
    next.request(&Command::RestoreDraft, false).unwrap();
    let saved = next.request(&Command::SaveDraft, false).unwrap();
    let id: taypeer_core::EntryId = serde_json::from_value(saved).unwrap();
    let entry = next.request(&Command::Entry(id.clone()), false).unwrap();
    assert_eq!(entry["title"], "PUBLIC unsaved title");
    let password = next.request(&Command::RevealPassword(id), false).unwrap();
    assert_eq!(password, "PUBLIC draft secret");
    next.control.invalidate(LockReason::Manual);
    assert_eq!(
        next.control.wait_closed().unwrap().draft,
        DraftDisposition::Preserved
    );
}

#[test]
fn draft_write_failure_closes_the_process_without_rewriting_the_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let sessions = SessionController::new(SessionPolicy::default());
    let client = open(&sessions, &path, true);
    edit(&client);
    let archive = std::fs::read(&path).unwrap();
    std::fs::create_dir(draft_path(&path)).unwrap();
    client.control.invalidate(LockReason::Background);
    let outcome = client.control.wait_closed().unwrap();
    assert_eq!(outcome.draft, DraftDisposition::Failed);
    assert!(outcome.error.is_some());
    assert!(client.request(&Command::Groups, false).is_err());
    assert_eq!(std::fs::read(&path).unwrap(), archive);
}
