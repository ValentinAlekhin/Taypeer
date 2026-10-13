//! Actual subprocess/supervisor coverage, with a child-only PUBLIC kernel-pipe fixture.
//! The child fixture does not claim Android authentication or send keys through callback IPC.
use super::*;
use crate::platform_worker::biometry::{
    BiometricBootAttempt, BiometricInputLease, BiometricInputs, PendingBiometricOpen,
};
use crate::session::{DraftDisposition, SessionPhase, SessionPolicy};

#[derive(Default)]
struct InputState {
    lease: Option<BiometricInputLease>,
    consumed: bool,
    retired: bool,
    held_writer: Option<File>,
}
pub(super) struct PublicInput {
    document: Arc<Document>,
    mode: String,
    state: Mutex<InputState>,
}
impl PublicInput {
    pub(super) fn new(document: Arc<Document>, mode: String) -> Self {
        Self {
            document,
            mode,
            state: Mutex::new(InputState::default()),
        }
    }
}
impl BiometricInputs for PublicInput {
    fn arm(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError> {
        let snapshot = self
            .document
            .snapshot()
            .map_err(taypeer_services::ServiceError::from)?;
        let scope = DatabaseService::archive_key_scope(&snapshot, self.document.working_copy())?;
        if scope != lease.scope.managed || lease.scope.generation == 0 {
            return Err(RuntimeError::Protocol);
        }
        let mut state = self.state.lock().map_err(|_| RuntimeError::Transport)?;
        if state.lease.is_some() || state.retired {
            return Err(RuntimeError::Protocol);
        }
        state.lease = Some(lease.clone());
        Ok(())
    }
    fn check(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError> {
        let state = self.state.lock().map_err(|_| RuntimeError::Transport)?;
        if state.retired || state.lease.as_ref() != Some(lease) {
            return Err(RuntimeError::Closed);
        }
        Ok(())
    }
    fn consume(&self, lease: &BiometricInputLease) -> Result<Box<dyn Read + Send>, RuntimeError> {
        self.check(lease)?;
        let mut state = self.state.lock().map_err(|_| RuntimeError::Transport)?;
        if state.consumed {
            return Err(RuntimeError::Protocol);
        }
        state.consumed = true;
        let (reader, writer) = rustix::pipe::pipe().map_err(|_| RuntimeError::Transport)?;
        let mut writer = File::from(writer);
        if self.mode == "blocked" {
            // Deliberately broken private provider: hold the empty writer until
            // actual OS termination. Independent supervisor must remain bounded.
            state.held_writer = Some(writer);
            let path =
                std::env::var_os("TAYPEER_PUBLIC_BIOMETRIC_READ").ok_or(RuntimeError::Protocol)?;
            std::fs::write(path, b"PUBLIC key read started")
                .map_err(|_| RuntimeError::Transport)?;
        } else {
            // PUBLIC-only key remains entirely inside this plaintext child. This
            // narrow fixture validates Runtime custody/authentication, not OS auth.
            let snapshot = self
                .document
                .snapshot()
                .map_err(taypeer_services::ServiceError::from)?;
            let key =
                DatabaseService::authenticate_archive_credentials(&snapshot, PASSWORD.as_bytes())?;
            writer
                .write_all(key.secret_bytes())
                .map_err(|_| RuntimeError::Transport)?;
            drop(writer);
        }
        Ok(Box::new(File::from(reader)))
    }
    fn retire(&self, lease: &BiometricInputLease) {
        if let Ok(mut state) = self.state.lock()
            && state.lease.as_ref() == Some(lease)
        {
            state.retired = true;
            state.held_writer = None;
        }
    }
}
struct BiometricLauncher<'a> {
    launcher: &'a Launcher,
    mode: &'a str,
}
impl ProcessLauncher for BiometricLauncher<'_> {
    fn launch(&self) -> Result<ProcessConnection, RuntimeError> {
        self.launcher.launch_with_biometry(Some(self.mode))
    }
}
fn fixture() -> Fixture {
    let fixture = Fixture::new();
    let mut worker = fixture.open(PASSWORD, true).unwrap();
    edit(&mut worker, "PUBLIC pending biometric saved entry");
    let input = editor(&mut worker);
    save(
        &mut worker,
        &input,
        taypeer_services::new_operation_id().unwrap(),
    )
    .unwrap();
    worker.close().unwrap();
    fixture.host.register_working_copy(&fixture.path).unwrap();
    fixture
}
fn begin(fixture: &Fixture, mode: &str) -> PendingBiometricOpen {
    let snapshot = fixture.launcher.writer.snapshot().unwrap();
    let managed = DatabaseService::archive_key_scope(
        &snapshot,
        fixture.launcher.writer.working_copy().unwrap(),
    )
    .unwrap();
    fixture
        .host
        .begin_platform_biometric_open(
            &BiometricLauncher {
                launcher: &fixture.launcher,
                mode,
            },
            BiometricBootAttempt {
                namespace: Digest::of(b"PUBLIC supervised biometric profile"),
                managed,
                attempt: Digest::object(
                    b"PUBLIC pending attempt",
                    &taypeer_services::new_operation_id().unwrap(),
                )
                .unwrap(),
                input: Digest::object(
                    b"PUBLIC pending input",
                    &taypeer_services::new_operation_id().unwrap(),
                )
                .unwrap(),
            },
        )
        .unwrap()
}
fn authors(fixture: &Fixture) -> usize {
    fixture.launcher.faults.author_calls.load(Ordering::Acquire)
}

#[test]
fn pending_challenge_is_actual_opening_and_exact_completion_attaches_the_ordinary_worker() {
    let fixture = fixture();
    let before = std::fs::read(&fixture.path).unwrap();
    let catalog = fixture.host.working_copies().unwrap();
    let initial_authors = authors(&fixture);
    let pending = begin(&fixture, "valid");
    let control = pending.control();
    let challenge = pending.challenge().clone();
    assert_eq!(control.status().phase, SessionPhase::Opening);
    assert!(control.status().database.is_none());
    assert!(!control.is_open());
    assert_eq!(challenge.scope.generation, control.status().generation);
    assert_eq!(authors(&fixture), initial_authors);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    let mut worker = pending.complete(&challenge).unwrap();
    assert_eq!(worker.session_status().phase, SessionPhase::Open);
    assert_eq!(worker.database_id(), &challenge.scope.managed.database);
    assert_eq!(authors(&fixture), initial_authors + 1);
    let groups = worker.request(&Command::Groups).unwrap();
    assert_eq!(groups.as_array().unwrap().len(), 1);
    assert!(matches!(
        worker.platform_biometric_scope(),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Unauthorized
        ))
    ));
    assert_eq!(fixture.host.working_copies().unwrap(), catalog);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    worker.close().unwrap();
    assert_eq!(control.status().phase, SessionPhase::Closed);
}

#[test]
fn opening_close_drop_background_and_idle_are_real_terminal_supervisor_transitions() {
    let fixture = fixture();
    let before = std::fs::read(&fixture.path).unwrap();
    let initial_authors = authors(&fixture);
    let mut pending = begin(&fixture, "valid");
    let first_generation = pending.challenge().scope.generation;
    let outcome = pending.close_report().unwrap();
    assert_eq!(outcome.termination, Termination::Graceful);
    assert_eq!(outcome.draft, DraftDisposition::Preserved);
    assert_eq!(pending.control().status().phase, SessionPhase::Closed);
    let pending = begin(&fixture, "valid");
    assert_ne!(pending.challenge().scope.generation, first_generation);
    let control = pending.control();
    drop(pending);
    assert_eq!(
        control.wait_closed().unwrap().reason,
        LockReason::HostExited
    );
    let pending = begin(&fixture, "valid");
    let lease = pending.challenge().clone();
    let control = pending.control();
    fixture.host.sessions().lock_all(LockReason::Background);
    assert!(matches!(
        control.status().phase,
        SessionPhase::Closing | SessionPhase::Closed
    ));
    assert!(pending.complete(&lease).is_err());
    assert_eq!(
        control.wait_closed().unwrap().reason,
        LockReason::Background
    );
    fixture.host.sessions().activity().touch();
    let mut pending = begin(&fixture, "valid");
    // This is genuine PUBLIC simulated foreground input, not background progress.
    fixture.host.sessions().activity().touch();
    fixture
        .host
        .sessions()
        .set_policy(SessionPolicy::new(1).unwrap());
    wait(|| pending.control().status().phase != SessionPhase::Opening);
    let outcome = pending.close_report().unwrap();
    assert_eq!(outcome.reason, LockReason::Idle);
    assert_eq!(outcome.termination, Termination::Graceful);
    assert_eq!(authors(&fixture), initial_authors);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
}

#[test]
fn changed_or_foreign_challenge_cannot_acquire_author_or_create_a_late_open_worker() {
    let fixture = fixture();
    let initial_authors = authors(&fixture);
    let pending = begin(&fixture, "valid");
    let control = pending.control();
    let mut wrong = pending.challenge().clone();
    wrong.input = Digest::of(b"PUBLIC foreign private input");
    assert!(matches!(
        pending.complete(&wrong),
        Err(RuntimeError::Protocol)
    ));
    control.wait_closed().unwrap();
    assert_eq!(authors(&fixture), initial_authors);
    let pending = begin(&fixture, "valid");
    let lease = pending.challenge().clone();
    let control = pending.control();
    let mut service = DatabaseService::new();
    let session = fixture
        .host
        .open_local(&mut service, &fixture.path, PASSWORD.as_bytes(), None)
        .unwrap();
    service
        .create_group(
            &session,
            "PUBLIC snapshot changed during prompt".into(),
            None,
            &taypeer_services::new_operation_id().unwrap(),
        )
        .unwrap();
    service.lock(&session).unwrap();
    let changed = std::fs::read(&fixture.path).unwrap();
    assert!(matches!(
        pending.complete(&lease),
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(StorageError::Changed)
        ))
    ));
    control.wait_closed().unwrap();
    assert_eq!(authors(&fixture), initial_authors);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), changed);
}

#[test]
fn blocked_private_read_cannot_delay_revocation_or_actual_forced_process_exit() {
    let fixture = fixture();
    let before = std::fs::read(&fixture.path).unwrap();
    let initial_authors = authors(&fixture);
    let pending = begin(&fixture, "blocked");
    let lease = pending.challenge().clone();
    let control = pending.control();
    let completion = std::thread::spawn(move || pending.complete(&lease));
    let started = fixture
        .launcher
        .directory
        .join("PUBLIC private read started");
    wait(|| started.exists());
    let at = Instant::now();
    control.invalidate(LockReason::Background);
    assert!(at.elapsed() < Duration::from_millis(100));
    assert!(matches!(
        completion.join().unwrap(),
        Err(RuntimeError::OperationInterrupted(LockReason::Background))
    ));
    assert_eq!(
        control.wait_closed().unwrap().termination,
        Termination::Forced
    );
    assert!(at.elapsed() < Duration::from_secs(3));
    assert_eq!(authors(&fixture), initial_authors);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    let mut fallback = fixture.open(PASSWORD, false).unwrap();
    assert!(fallback.request(&Command::DatabaseInfo).is_ok());
    fallback.close().unwrap();
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
}
