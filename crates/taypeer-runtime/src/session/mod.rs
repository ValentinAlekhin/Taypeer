//! Independent lifetime supervision. Command I/O never owns the process-control lock.
mod process;
mod settings;
#[cfg(test)]
mod tests;
pub(crate) use process::ProcessControl;
pub use settings::SessionSettings;
pub use taypeer_core::SessionPolicy;

use crate::RuntimeError;
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, Weak};
use std::time::{Duration, Instant};
use taypeer_core::DatabaseId;

pub(crate) const CLOSE_GRACE: Duration = Duration::from_secs(2);
pub(crate) const POLL: Duration = Duration::from_millis(20);

/// Content-free reason for invalidating an unlocked or opening session.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum LockReason {
    /// Explicit user command.
    Manual,
    /// Shared monotonic inactivity deadline.
    Idle,
    /// Platform will sleep.
    Sleep,
    /// Platform session was locked.
    SystemLocked,
    /// Android application left the foreground.
    Background,
    /// Admission, epoch or authority became invalid.
    Authority,
    /// Host disappeared or shut down.
    HostExited,
    /// Private protocol or process failed.
    Transport,
}
/// Observable lifetime, never inferred from a pending command's completion.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SessionPhase {
    /// Authentication is still running.
    Opening,
    /// Commands may access this generation.
    Open,
    /// Access has been revoked; bounded cleanup is in progress.
    Closing,
    /// Operating-system exit has been observed.
    Closed,
}
/// Whether the child cooperated with shutdown.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Termination {
    /// Worker acknowledged its lock command and exited.
    Graceful,
    /// Supervisor killed the child after its grace period.
    Forced,
    /// Child exited without a lock acknowledgement.
    Exited,
}
/// Durable draft acknowledgement is separate from process termination.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DraftDisposition {
    /// The worker confirmed its normal draft policy (including deferred read-only drafts).
    Preserved,
    /// Saving the draft failed explicitly.
    Failed,
    /// No acknowledgement; the last durable draft may still exist.
    Unconfirmed,
}
/// Result retained after access has been revoked, including failed graceful shutdowns.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LockOutcome {
    /// Trigger of the first invalidation.
    pub reason: LockReason,
    /// How the child ended.
    pub termination: Termination,
    /// What is known about the encrypted draft.
    pub draft: DraftDisposition,
    /// Sanitized failure, if one was acknowledged.
    pub error: Option<RuntimeError>,
}
/// Nonsecret session state for client display and automation.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct SessionStatus {
    /// Absent until opening authenticates the database identity.
    pub database: Option<DatabaseId>,
    /// Unique within this controller, never reused after reauthentication.
    pub generation: u64,
    /// Current access state.
    pub phase: SessionPhase,
    /// First invalidating event, if any.
    pub reason: Option<LockReason>,
    /// Completion details, once the child has ended.
    pub outcome: Option<LockOutcome>,
}

trait Clock: Send + Sync {
    fn now(&self) -> Duration;
}
struct Monotonic(Instant);
impl Clock for Monotonic {
    fn now(&self) -> Duration {
        self.0.elapsed()
    }
}
struct Timer {
    policy: SessionPolicy,
    last: Duration,
    epoch: u64,
    reason: Option<LockReason>,
    next: u64,
    stopped: bool,
    processes: Vec<Weak<ProcessControl>>,
}
pub(crate) struct Inner {
    timer: Mutex<Timer>,
    clock: Arc<dyn Clock>,
}
struct Owner(Arc<Inner>);
impl Drop for Owner {
    fn drop(&mut self) {
        if let Ok(mut timer) = self.0.timer.lock() {
            timer.stopped = true;
        }
        self.0.invalidate(LockReason::HostExited);
    }
}
/// Shared policy and supervisor for all database processes in one application instance.
#[derive(Clone)]
pub struct SessionController(Arc<Owner>);
/// Weak, nonsecret input hook. An abandoned terminal reader cannot keep sessions alive.
#[derive(Clone)]
pub struct ActivityHandle(Weak<Inner>);
impl SessionController {
    /// Start independent supervision; no profile, credentials or database is opened.
    pub fn new(policy: SessionPolicy) -> Self {
        Self::with_clock(policy, Arc::new(Monotonic(Instant::now())))
    }
    fn with_clock(policy: SessionPolicy, clock: Arc<dyn Clock>) -> Self {
        let inner = Arc::new(Inner {
            timer: Mutex::new(Timer {
                policy,
                last: clock.now(),
                epoch: 0,
                reason: None,
                next: 1,
                stopped: false,
                processes: Vec::new(),
            }),
            clock,
        });
        let watch = Arc::clone(&inner);
        std::thread::spawn(move || {
            loop {
                watch.expire();
                let (stopped, processes) = watch.processes();
                for process in &processes {
                    process.poll();
                }
                if stopped && processes.iter().all(|p| p.finished()) {
                    break;
                }
                std::thread::sleep(POLL);
            }
        });
        Self(Arc::new(Owner(inner)))
    }
    /// Input-only hook; background requests must not call it.
    pub fn activity(&self) -> ActivityHandle {
        ActivityHandle(Arc::downgrade(&self.0.0))
    }
    /// Current nonsecret policy.
    pub fn policy(&self) -> SessionPolicy {
        self.0
            .0
            .timer
            .lock()
            .expect("timer contains no fallible callbacks")
            .policy
    }
    /// Apply a validated local preference without counting its completion as new input.
    pub fn set_policy(&self, policy: SessionPolicy) {
        let inner = &self.0.0;
        let mut timer = inner
            .timer
            .lock()
            .expect("timer contains no fallible callbacks");
        let now = inner.clock.now();
        inner.expire_locked(&mut timer, now);
        timer.policy = policy;
        inner.expire_locked(&mut timer, now);
    }
    /// Revoke all sessions immediately; their two-second shutdowns run concurrently.
    /// Sleep/SystemLocked/Background are adapter hooks, not subscriptions to native events.
    pub fn lock_all(&self, reason: LockReason) {
        self.0.0.invalidate(reason);
    }
    /// Read current states without waiting for command IPC or disk work.
    pub fn statuses(&self) -> Vec<SessionStatus> {
        self.0.0.expire();
        self.0.0.processes().1.iter().map(|p| p.status()).collect()
    }
    pub(crate) fn register(&self, process: &Arc<ProcessControl>) -> Result<u64, RuntimeError> {
        self.0.0.expire();
        let mut timer = self.0.0.timer.lock().map_err(|_| RuntimeError::Closed)?;
        if timer.stopped {
            return Err(RuntimeError::Closed);
        }
        let generation = timer.next;
        timer.next = generation.checked_add(1).ok_or(RuntimeError::Closed)?;
        timer.processes.retain(|p| p.strong_count() > 0);
        timer.processes.push(Arc::downgrade(process));
        Ok(generation)
    }
}
impl ActivityHandle {
    pub(crate) fn interrupted(&self, reason: LockReason) {
        if let Some(inner) = self.0.upgrade()
            && let Ok(mut timer) = inner.timer.lock()
        {
            timer.epoch = timer.epoch.saturating_add(1);
            timer.reason = Some(reason);
        }
    }
    /// Most recent controller-wide invalidation reason, for cancelling pending input.
    pub fn reason(&self) -> Option<LockReason> {
        self.0
            .upgrade()
            .and_then(|inner| inner.timer.lock().ok().and_then(|timer| timer.reason))
    }
    /// Expire old sessions first, then record genuine input. Never reopens a session.
    pub fn touch(&self) {
        if let Some(inner) = self.0.upgrade()
            && let Ok(mut timer) = inner.timer.lock()
        {
            let now = inner.clock.now();
            inner.expire_locked(&mut timer, now);
            timer.last = now;
        }
    }
    /// Invalidation generation for cancelling a pending prompt or rejecting stale output.
    pub fn epoch(&self) -> u64 {
        self.0.upgrade().map_or(u64::MAX, |inner| {
            inner.expire();
            inner.timer.lock().map_or(u64::MAX, |timer| timer.epoch)
        })
    }
    pub(crate) fn expire(&self) {
        if let Some(inner) = self.0.upgrade() {
            inner.expire();
        }
    }
}
impl Inner {
    fn processes(&self) -> (bool, Vec<Arc<ProcessControl>>) {
        self.timer.lock().map_or((true, Vec::new()), |timer| {
            (
                timer.stopped,
                timer.processes.iter().filter_map(Weak::upgrade).collect(),
            )
        })
    }
    fn expire(&self) {
        let mut timer = self
            .timer
            .lock()
            .expect("timer contains no fallible callbacks");
        self.expire_locked(&mut timer, self.clock.now());
    }
    fn expire_locked(&self, timer: &mut Timer, now: Duration) {
        if !timer.stopped && now.saturating_sub(timer.last) >= timer.policy.idle_duration() {
            timer.last = now;
            Self::revoke_locked(timer, LockReason::Idle);
        }
    }
    fn revoke_locked(timer: &mut Timer, reason: LockReason) {
        timer.epoch = timer.epoch.saturating_add(1);
        timer.reason = Some(reason);
        // Publish epoch and revoke every access gate in one critical section.
        // No command, pipe or storage callback can hold either of these locks.
        for process in timer.processes.iter().filter_map(Weak::upgrade) {
            process.revoke(reason);
        }
    }
    fn invalidate(&self, reason: LockReason) {
        let mut timer = self
            .timer
            .lock()
            .expect("timer contains no fallible callbacks");
        Self::revoke_locked(&mut timer, reason);
    }
}
