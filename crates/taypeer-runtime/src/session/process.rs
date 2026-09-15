use super::*;
use crate::process::Job;
use std::{
    process::Child,
    sync::{
        atomic::{AtomicU64, Ordering},
        mpsc::SyncSender,
    },
};

struct State {
    child: Option<Child>,
    database: Option<DatabaseId>,
    phase: SessionPhase,
    closing: Option<(LockReason, Instant)>,
    sent: bool,
    killed: bool,
    acknowledgement: Option<Result<(), RuntimeError>>,
    outcome: Option<LockOutcome>,
}
pub(crate) struct ProcessControl {
    state: Mutex<State>,
    generation: AtomicU64,
    jobs: SyncSender<Job>,
    pub(crate) activity: ActivityHandle,
}
impl ProcessControl {
    pub(crate) fn take_pipes(
        &self,
    ) -> Result<(std::process::ChildStdin, std::process::ChildStdout), RuntimeError> {
        let mut state = self.state.lock().map_err(|_| RuntimeError::Closed)?;
        let child = state.child.as_mut().ok_or(RuntimeError::Closed)?;
        Ok((
            child.stdin.take().ok_or(RuntimeError::Transport)?,
            child.stdout.take().ok_or(RuntimeError::Transport)?,
        ))
    }
    pub(crate) fn new(child: Child, jobs: SyncSender<Job>, activity: ActivityHandle) -> Self {
        Self {
            state: Mutex::new(State {
                child: Some(child),
                database: None,
                phase: SessionPhase::Opening,
                closing: None,
                sent: false,
                killed: false,
                acknowledgement: None,
                outcome: None,
            }),
            generation: AtomicU64::new(0),
            jobs,
            activity,
        }
    }
    pub(crate) fn set_generation(&self, generation: u64) {
        self.generation.store(generation, Ordering::Release);
    }
    pub(crate) fn opened(&self, database: DatabaseId) -> Result<(), RuntimeError> {
        self.check(true)?;
        let mut state = self.state.lock().map_err(|_| RuntimeError::Closed)?;
        if state.phase != SessionPhase::Opening {
            return Err(RuntimeError::Closed);
        }
        state.database = Some(database);
        state.phase = SessionPhase::Open;
        Ok(())
    }
    pub(crate) fn check(&self, opening: bool) -> Result<(), RuntimeError> {
        self.activity.expire();
        let state = self.state.lock().map_err(|_| RuntimeError::Closed)?;
        if state.phase == SessionPhase::Open || (opening && state.phase == SessionPhase::Opening) {
            Ok(())
        } else {
            Err(RuntimeError::SessionClosed(
                state.closing.map_or(LockReason::Transport, |c| c.0),
            ))
        }
    }
    pub(crate) fn invalidate(&self, reason: LockReason) {
        if self.revoke(reason) && matches!(reason, LockReason::Authority | LockReason::Transport) {
            // Cancel owned prompts/output too. Ordinary command-driven close does
            // not invalidate the command's own nonsecret completion report.
            self.activity.interrupted(reason);
        }
        self.poll();
    }
    // Called under the timer lock too: only mutate access, never perform I/O.
    pub(crate) fn revoke(&self, reason: LockReason) -> bool {
        if let Ok(mut state) = self.state.lock()
            && matches!(state.phase, SessionPhase::Opening | SessionPhase::Open)
        {
            state.phase = SessionPhase::Closing;
            state.closing = Some((reason, Instant::now()));
            return true;
        }
        false
    }
    pub(crate) fn acknowledge(&self, result: Result<(), RuntimeError>) {
        if let Ok(mut state) = self.state.lock() {
            state.acknowledgement = Some(result);
        }
    }
    pub(crate) fn poll(&self) {
        let Ok(mut state) = self.state.lock() else {
            return;
        };
        if state.phase == SessionPhase::Closed {
            return;
        }
        match state
            .child
            .as_mut()
            .expect("child retained until control drops")
            .try_wait()
        {
            Ok(Some(_)) => {
                let unexpected = state.closing.is_none();
                let (reason, _) = *state
                    .closing
                    .get_or_insert((LockReason::Transport, Instant::now()));
                // An exited child may still have its lock response buffered in stdout.
                // Let the pipe actor consume it before declaring the draft unconfirmed.
                if state.acknowledgement.is_none()
                    && state
                        .closing
                        .is_some_and(|(_, at)| at.elapsed() < CLOSE_GRACE)
                {
                    state.phase = SessionPhase::Closing;
                    if !state.sent {
                        state.sent = self.jobs.try_send(Job::Close).is_ok();
                    }
                    drop(state);
                    if unexpected {
                        self.activity.interrupted(LockReason::Transport);
                    }
                    return;
                }
                state.phase = SessionPhase::Closed;
                state.outcome = Some(LockOutcome {
                    reason,
                    termination: if state.killed {
                        Termination::Forced
                    } else if matches!(
                        state.acknowledgement,
                        Some(Ok(()) | Err(RuntimeError::Service(_)))
                    ) {
                        Termination::Graceful
                    } else {
                        Termination::Exited
                    },
                    draft: match state.acknowledgement {
                        Some(Ok(())) => DraftDisposition::Preserved,
                        Some(Err(RuntimeError::Service(_))) => DraftDisposition::Failed,
                        Some(Err(_)) => DraftDisposition::Unconfirmed,
                        None => DraftDisposition::Unconfirmed,
                    },
                    error: state.acknowledgement.and_then(Result::err),
                });
                return;
            }
            Err(_) => {
                state.phase = SessionPhase::Closing;
                state
                    .closing
                    .get_or_insert((LockReason::Transport, Instant::now()));
            }
            Ok(None) => {}
        }
        if let Some((_, since)) = state.closing {
            if !state.sent {
                state.sent = self.jobs.try_send(Job::Close).is_ok();
            }
            if !state.killed && since.elapsed() >= CLOSE_GRACE {
                // Only this short control lock owns Child. No pipe or filesystem callback can hold it.
                state.killed = state
                    .child
                    .as_mut()
                    .expect("child retained until control drops")
                    .kill()
                    .is_ok();
            }
        }
    }
    pub(crate) fn status(&self) -> SessionStatus {
        let state = self
            .state
            .lock()
            .expect("process control never calls application code");
        SessionStatus {
            database: state.database.clone(),
            generation: self.generation.load(Ordering::Acquire),
            phase: state.phase,
            reason: state.closing.map(|c| c.0),
            outcome: state.outcome,
        }
    }
    pub(crate) fn finished(&self) -> bool {
        self.status().phase == SessionPhase::Closed
    }
    pub(crate) fn wait_closed(&self) -> Result<LockOutcome, RuntimeError> {
        let since = self
            .state
            .lock()
            .map_err(|_| RuntimeError::Closed)?
            .closing
            .map_or_else(Instant::now, |(_, since)| since);
        let until = since + CLOSE_GRACE + Duration::from_millis(200);
        loop {
            self.poll();
            if let Some(outcome) = self.status().outcome {
                return Ok(outcome);
            }
            if Instant::now() >= until {
                return Err(RuntimeError::ShutdownUnconfirmed);
            }
            std::thread::sleep(POLL);
        }
    }
}

impl Drop for ProcessControl {
    fn drop(&mut self) {
        let state = self
            .state
            .get_mut()
            .unwrap_or_else(|error| error.into_inner());
        if state.phase != SessionPhase::Closed
            && let Some(mut child) = state.child.take()
        {
            // Covers registration/spawn failures too. Reaping cannot block the caller.
            let _ = child.kill();
            std::thread::spawn(move || {
                let _ = child.wait();
            });
        }
    }
}
