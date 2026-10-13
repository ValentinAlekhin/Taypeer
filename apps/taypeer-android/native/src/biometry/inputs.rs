//! One private supervised worker owns at most one accepted key input at a time.
//! Retired/consumed attempt IDs are never evicted during that worker's lifetime.
use super::key_fd;
use crate::AndroidError;
use std::{collections::BTreeSet, io::Read, os::fd::RawFd, sync::Mutex};
use taypeer_core::DatabaseId;
use taypeer_runtime::{
    RuntimeError,
    platform_worker::biometry::{BiometricAttemptScope, BiometricInputLease, BiometricInputs},
};
use taypeer_trust::Digest;

const MAX_ATTEMPTS: usize = 64;
type KeyReader = Box<dyn Read + Send>;

struct WorkerBasis {
    namespace: Digest,
    working_copy: Digest,
    database: DatabaseId,
    root: Digest,
    generation: u64,
}
impl WorkerBasis {
    fn from_scope(scope: &BiometricAttemptScope) -> Self {
        Self {
            namespace: scope.namespace,
            working_copy: scope.managed.working_copy,
            database: scope.managed.database.clone(),
            root: scope.managed.root,
            generation: scope.generation,
        }
    }
    fn matches(&self, scope: &BiometricAttemptScope) -> bool {
        self.namespace == scope.namespace
            && self.working_copy == scope.managed.working_copy
            && self.database == scope.managed.database
            && self.root == scope.managed.root
            && self.generation == scope.generation
    }
}

enum Phase {
    Waiting,
    Installed(KeyReader),
    Consumed,
    Retired,
}
struct Slot {
    lease: BiometricInputLease,
    phase: Phase,
}
#[derive(Default)]
struct State {
    basis: Option<WorkerBasis>,
    // Includes every admitted attempt, not just the latest terminal attempt.
    // Full capacity refuses another attempt instead of resurrecting an old one.
    attempts: BTreeSet<Digest>,
    slot: Option<Slot>,
}

/// Installed once by the trusted isolated bootstrap. No key bytes, raw FD getter,
/// public constructor, serialization or UniFFI action is exposed to UI callers.
#[derive(Default, uniffi::Object)]
pub struct PrivateBiometricInputs {
    state: Mutex<State>,
}
#[uniffi::export]
impl PrivateBiometricInputs {
    /// Fixed isolated-service bootstrap owner. It exposes no key reader to UI.
    #[uniffi::constructor(name = "new")]
    pub fn bootstrap() -> std::sync::Arc<Self> {
        std::sync::Arc::new(Self::new())
    }
    /// Private Binder handoff; caller retains its original until owned-dup ACK.
    pub fn install_input(
        &self,
        spec: super::BiometricInputSpec,
        fd: i32,
    ) -> Result<(), AndroidError> {
        self.install(&spec.checked()?, fd)
    }
    /// Independent private service revocation, never a command-pipe request.
    pub fn retire_input(&self, spec: super::BiometricInputSpec) -> Result<(), AndroidError> {
        use taypeer_runtime::platform_worker::biometry::BiometricInputs;
        self.retire(&spec.checked()?);
        Ok(())
    }
}
impl PrivateBiometricInputs {
    pub(super) fn new() -> Self {
        Self::default()
    }

    /// The private Binder caller must hold its original PFD until this returns.
    /// Only an already armed exact lease can receive a duplicate. Cancellation
    /// racing the out-of-lock import closes that duplicate without an ACK.
    pub(super) fn install(
        &self,
        lease: &BiometricInputLease,
        held_fd: RawFd,
    ) -> Result<(), AndroidError> {
        self.install_with(lease, held_fd, key_fd::import_private)
    }

    fn install_with(
        &self,
        lease: &BiometricInputLease,
        held_fd: RawFd,
        duplicate: impl FnOnce(RawFd) -> Result<KeyReader, AndroidError>,
    ) -> Result<(), AndroidError> {
        {
            let state = self.state.lock().map_err(|_| AndroidError::Runtime)?;
            require_waiting(&state, lease)?;
        }
        let owned = duplicate(held_fd)?;
        let mut state = self.state.lock().map_err(|_| AndroidError::Runtime)?;
        if let Err(error) = require_waiting(&state, lease) {
            drop(state);
            drop(owned);
            return Err(error);
        }
        let Some(slot) = &mut state.slot else {
            // require_waiting checked this same mutex-protected state.
            drop(state);
            drop(owned);
            return Err(AndroidError::InvalidOptions);
        };
        slot.phase = Phase::Installed(owned);
        Ok(())
    }
}

fn valid_lease(lease: &BiometricInputLease) -> bool {
    // Exact projection of the shared trust wire guard, not a new ID policy.
    lease.scope.generation != 0
        && !lease.scope.managed.database.as_str().is_empty()
        && lease.scope.managed.database.as_str().len() <= 128
}
fn require_waiting(state: &State, lease: &BiometricInputLease) -> Result<(), AndroidError> {
    let slot = state.slot.as_ref().ok_or(AndroidError::InvalidOptions)?;
    if &slot.lease != lease {
        return Err(AndroidError::InvalidOptions);
    }
    match &slot.phase {
        Phase::Waiting => Ok(()),
        Phase::Retired => Err(AndroidError::Runtime),
        Phase::Installed(_) | Phase::Consumed => Err(AndroidError::InvalidOptions),
    }
}
fn checked_slot<'a>(
    state: &'a State,
    lease: &BiometricInputLease,
) -> Result<&'a Slot, RuntimeError> {
    let slot = state.slot.as_ref().ok_or(RuntimeError::Closed)?;
    if &slot.lease != lease {
        return Err(RuntimeError::Protocol);
    }
    if matches!(slot.phase, Phase::Retired) {
        return Err(RuntimeError::Closed);
    }
    Ok(slot)
}

impl BiometricInputs for PrivateBiometricInputs {
    fn arm(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError> {
        if !valid_lease(lease) {
            return Err(RuntimeError::Protocol);
        }
        let mut state = self.state.lock().map_err(|_| RuntimeError::Transport)?;
        if state
            .basis
            .as_ref()
            .is_some_and(|basis| !basis.matches(&lease.scope))
            || state.attempts.contains(&lease.scope.attempt)
            || state.attempts.len() >= MAX_ATTEMPTS
            || state
                .slot
                .as_ref()
                .is_some_and(|slot| !matches!(slot.phase, Phase::Retired))
        {
            return Err(RuntimeError::Protocol);
        }
        if state.basis.is_none() {
            state.basis = Some(WorkerBasis::from_scope(&lease.scope));
        }
        state.attempts.insert(lease.scope.attempt);
        state.slot = Some(Slot {
            lease: lease.clone(),
            phase: Phase::Waiting,
        });
        Ok(())
    }

    fn check(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError> {
        let state = self.state.lock().map_err(|_| RuntimeError::Transport)?;
        checked_slot(&state, lease)?;
        Ok(())
    }

    fn consume(&self, lease: &BiometricInputLease) -> Result<KeyReader, RuntimeError> {
        let mut state = self.state.lock().map_err(|_| RuntimeError::Transport)?;
        let slot = checked_slot(&state, lease)?;
        if !matches!(slot.phase, Phase::Installed(_)) {
            return Err(RuntimeError::Protocol);
        }
        let slot = state.slot.as_mut().ok_or(RuntimeError::Closed)?;
        match std::mem::replace(&mut slot.phase, Phase::Consumed) {
            Phase::Installed(reader) => Ok(reader),
            Phase::Waiting | Phase::Consumed | Phase::Retired => Err(RuntimeError::Protocol),
        }
    }

    fn retire(&self, lease: &BiometricInputLease) {
        if !valid_lease(lease) {
            return;
        }
        let discarded = {
            let Ok(mut state) = self.state.lock() else {
                return;
            };
            if state
                .basis
                .as_ref()
                .is_some_and(|basis| !basis.matches(&lease.scope))
            {
                return;
            }
            if state.attempts.len() < MAX_ATTEMPTS {
                // Before-arrival cancellation must prevent a later arm as well.
                state.attempts.insert(lease.scope.attempt);
            }
            state
                .slot
                .as_mut()
                .filter(|slot| &slot.lease == lease)
                .map(|slot| std::mem::replace(&mut slot.phase, Phase::Retired))
        };
        // Descriptor close does not run under the independent lifecycle mutex.
        drop(discarded);
    }
}

#[cfg(test)]
mod tests;
