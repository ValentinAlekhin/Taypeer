use super::*;
use crate::{
    cipher_ipc::IoValue,
    session::{
        DraftDisposition, LockReason, SessionController, SessionPhase, SessionPolicy, Termination,
    },
};
use std::{
    path::Path,
    process::{Command as Process, Stdio},
    time::{Duration, Instant},
};

struct NoIo;
impl CallbackHandler for NoIo {
    fn handle(&mut self, _: IoRequest) -> Result<IoValue, RuntimeError> {
        Err(RuntimeError::Protocol)
    }
}
fn launch(sessions: &SessionController, mode: &str, ready: &Path) -> Arc<Client> {
    launch_with(sessions, mode, ready, Box::new(NoIo))
}
fn launch_with(
    sessions: &SessionController,
    mode: &str,
    ready: &Path,
    callbacks: Box<dyn CallbackHandler>,
) -> Arc<Client> {
    let mut child = Process::new("python3")
        .args(["-u", "-c", include_str!("fixture.py"), mode])
        .arg(ready)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap();
    let input = child.stdin.take().unwrap();
    let output = BufReader::new(child.stdout.take().unwrap());
    let (jobs, receive) = mpsc::sync_channel(1);
    let control = Arc::new(ProcessControl::new(
        child,
        jobs.clone(),
        sessions.activity(),
    ));
    control.set_generation(sessions.register(&control).unwrap());
    let actor = PipeActor {
        input,
        output,
        callbacks,
        spool: tempfile::tempdir().unwrap(),
    };
    let running = Arc::clone(&control);
    std::thread::spawn(move || actor.run(receive, running));
    Arc::new(Client { control, jobs })
}

#[test]
fn stalled_ciphertext_callback_cannot_delay_access_revocation_or_process_exit() {
    struct Stalled {
        entered: mpsc::SyncSender<()>,
        release: mpsc::Receiver<()>,
        finished: mpsc::SyncSender<()>,
    }
    impl CallbackHandler for Stalled {
        fn handle(&mut self, request: IoRequest) -> Result<IoValue, RuntimeError> {
            assert!(matches!(request, IoRequest::Snapshot { .. }));
            self.entered.send(()).unwrap();
            // The parent owns only the ciphertext callback; it can outlive the child.
            self.release.recv_timeout(Duration::from_secs(10)).unwrap();
            self.finished.send(()).unwrap();
            Ok(IoValue::Unchanged)
        }
    }
    let (entered, started) = mpsc::sync_channel(1);
    let (release, resume) = mpsc::sync_channel(1);
    let (finished, ended) = mpsc::sync_channel(1);
    let sessions = SessionController::new(SessionPolicy::default());
    let directory = tempfile::tempdir().unwrap();
    let client = launch_with(
        &sessions,
        "io",
        &directory.path().join("ready"),
        Box::new(Stalled {
            entered,
            release: resume,
            finished,
        }),
    );
    let id = client.request(&"PUBLIC boot", true).unwrap();
    client
        .control
        .opened(serde_json::from_value(id).unwrap())
        .unwrap();
    let running = Arc::clone(&client);
    let request = std::thread::spawn(move || running.request(&Command::Groups, false));
    started.recv_timeout(Duration::from_secs(3)).unwrap();
    let epoch = sessions.activity().epoch();
    client.control.invalidate(LockReason::Authority);
    assert_ne!(sessions.activity().epoch(), epoch); // Pending foreground input is cancelled too.
    assert_eq!(
        request.join().unwrap(),
        Err(RuntimeError::OperationInterrupted(LockReason::Authority))
    );
    assert_eq!(
        client.control.wait_closed().unwrap().termination,
        Termination::Forced
    );
    release.send(()).unwrap();
    ended.recv_timeout(Duration::from_secs(3)).unwrap();
    assert!(client.control.check(false).is_err());
}
pub(crate) fn open(sessions: &SessionController, mode: &str, ready: &Path) -> Arc<Client> {
    let client = launch(sessions, mode, ready);
    let id = client.request(&"PUBLIC boot", true).unwrap();
    client
        .control
        .opened(serde_json::from_value(id).unwrap())
        .unwrap();
    client
}
fn await_ready(path: &Path) {
    let until = Instant::now() + Duration::from_secs(5);
    while !path.exists() {
        assert!(Instant::now() < until);
        std::thread::sleep(POLL);
    }
}

#[test]
fn unexpected_exit_releases_the_idle_pipe_actor() {
    let sessions = SessionController::new(SessionPolicy::default());
    let directory = tempfile::tempdir().unwrap();
    let client = open(&sessions, "exit_idle", &directory.path().join("ready"));
    let weak = Arc::downgrade(&client.control);
    let outcome = client.control.wait_closed().unwrap();
    assert_eq!(outcome.reason, LockReason::Transport);
    assert_eq!(outcome.termination, Termination::Exited);
    assert_eq!(outcome.draft, DraftDisposition::Unconfirmed);
    drop(client);
    let until = Instant::now() + Duration::from_secs(2);
    while weak.upgrade().is_some() {
        assert!(
            Instant::now() < until,
            "pipe actor retained the exited child"
        );
        std::thread::sleep(POLL);
    }
}

#[test]
fn hung_commands_and_partial_frames_cannot_delay_parallel_shutdown() {
    let sessions = SessionController::new(SessionPolicy::default());
    let directory = tempfile::tempdir().unwrap();
    let mut clients = Vec::new();
    let mut requests = Vec::new();
    for mode in ["hang", "partial"] {
        let ready = directory.path().join(mode);
        let client = open(&sessions, mode, &ready);
        let running = Arc::clone(&client);
        requests.push(std::thread::spawn(move || {
            running.request(&Command::Groups, false)
        }));
        await_ready(&ready);
        clients.push(client);
    }
    let began = Instant::now();
    sessions.lock_all(LockReason::Sleep);
    assert!(clients.iter().all(|c| c.control.check(false).is_err()));
    for request in requests {
        assert_eq!(
            request.join().unwrap(),
            Err(RuntimeError::OperationInterrupted(LockReason::Sleep))
        );
    }
    for client in &clients {
        let result = client.control.wait_closed().unwrap();
        assert_eq!(result.termination, Termination::Forced);
        assert_eq!(result.draft, DraftDisposition::Unconfirmed);
        assert_eq!(client.control.status().phase, SessionPhase::Closed);
    }
    assert!(began.elapsed() < Duration::from_secs(3));
}

#[test]
fn late_results_are_rejected_and_new_sessions_have_distinct_generations() {
    let sessions = SessionController::new(SessionPolicy::default());
    let directory = tempfile::tempdir().unwrap();
    let ready = directory.path().join("late");
    let old = open(&sessions, "late", &ready);
    let running = Arc::clone(&old);
    let request = std::thread::spawn(move || running.request(&Command::Groups, false));
    await_ready(&ready);
    sessions.lock_all(LockReason::SystemLocked);
    assert!(request.join().unwrap().is_err());
    let next = open(&sessions, "normal", &directory.path().join("next"));
    assert_ne!(
        old.control.status().generation,
        next.control.status().generation
    );
    assert_eq!(
        next.request(&Command::Groups, false).unwrap(),
        "PUBLIC revealed value"
    );
    assert!(old.control.check(false).is_err());
    assert_eq!(
        old.control.wait_closed().unwrap().draft,
        DraftDisposition::Preserved
    );
    next.control.invalidate(LockReason::Manual);
    assert_eq!(
        next.control.wait_closed().unwrap().termination,
        Termination::Graceful
    );
}

#[test]
fn failed_draft_acknowledgement_does_not_keep_access_open() {
    let sessions = SessionController::new(SessionPolicy::default());
    let directory = tempfile::tempdir().unwrap();
    let client = open(&sessions, "draft_error", &directory.path().join("ready"));
    client.control.invalidate(LockReason::Manual);
    let outcome = client.control.wait_closed().unwrap();
    assert_eq!(outcome.draft, DraftDisposition::Failed);
    assert!(outcome.error.is_some());
    assert!(client.request(&Command::Groups, false).is_err());
}

#[test]
fn opening_is_also_bounded_by_the_shared_idle_deadline() {
    let sessions = SessionController::new(SessionPolicy::new(1).unwrap());
    let directory = tempfile::tempdir().unwrap();
    let client = launch(&sessions, "opening", &directory.path().join("ready"));
    let began = Instant::now();
    assert_eq!(
        client.request(&"PUBLIC boot", true),
        Err(RuntimeError::OperationInterrupted(LockReason::Idle))
    );
    assert!(began.elapsed() < Duration::from_secs(2));
    assert_eq!(
        client.control.wait_closed().unwrap().termination,
        Termination::Forced
    );
}
