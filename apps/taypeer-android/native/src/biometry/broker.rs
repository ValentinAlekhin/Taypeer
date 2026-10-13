//! One fixed OS adapter and serialized durable owner for each registered copy.
use super::{
    BiometricAvailability, BiometricBindingView, BiometryFailure,
    port::{AttemptFence, OsBiometricKeyLease, OsBiometryPort},
    refusal,
    registry::{Binding, Generation, Registry, SetupState, WrapperRecord},
    registry_error,
};
use crate::{AndroidError, WorkingCopyView};
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::{
        Arc, Mutex, Weak,
        atomic::{AtomicU64, Ordering},
    },
};
use taypeer_runtime::{RuntimeHost, platform_worker::PlatformCipherWriter};
use taypeer_services::{DatabaseService, ManagedKeyScope};
use taypeer_trust::Digest;

pub(super) struct RegistrySlot {
    pub(super) registry: Mutex<Registry>,
    busy: AtomicU64,
    next: AtomicU64,
}
/// Serialization includes OS phases without holding a mutex across a prompt.
pub(super) struct Reservation {
    slot: Arc<RegistrySlot>,
    ticket: u64,
}
impl Reservation {
    pub(super) fn slot(&self) -> &RegistrySlot {
        &self.slot
    }
    pub(super) fn check(&self) -> Result<(), AndroidError> {
        if self.slot.busy.load(Ordering::Acquire) == self.ticket {
            Ok(())
        } else {
            Err(refusal(BiometryFailure::Cancelled))
        }
    }
    pub(super) fn release(&self) {
        // A late old handle never clears a newer actor's ticket.
        let _ =
            self.slot
                .busy
                .compare_exchange(self.ticket, 0, Ordering::AcqRel, Ordering::Acquire);
    }
}
impl Drop for Reservation {
    fn drop(&mut self) {
        self.release();
    }
}
pub(crate) struct Broker {
    runtime: Arc<RuntimeHost>,
    profile: PathBuf,
    pub(super) port: Arc<dyn OsBiometryPort>,
    registries: Mutex<BTreeMap<Digest, Arc<RegistrySlot>>>,
    attempts: Mutex<Vec<Weak<AttemptFence>>>,
    revocation: AtomicU64,
}
impl Broker {
    pub(super) fn discard_orphan(&self, candidate: WrapperRecord) {
        let port = Arc::clone(&self.port);
        // Only a validated fresh candidate from Prepared state is queued here.
        // No pending publication or active generation is eligible for cleanup.
        // Cancellation's Native fence has already fired; Keystore IO never
        // delays that fence or takes a worker/registry mutex.
        let _ = std::thread::Builder::new()
            .name("biometric-orphan-cleanup".into())
            .spawn(move || {
                if let Ok(binding) = BiometricBindingView::from_binding(candidate.binding()) {
                    let _ = port.delete_key(
                        binding,
                        candidate.generation().to_string(),
                        candidate.alias().to_owned(),
                    );
                }
            });
    }
    pub(crate) fn new(runtime: Arc<RuntimeHost>, port: Arc<dyn OsBiometryPort>) -> Arc<Self> {
        Arc::new(Self {
            profile: runtime.profile().directory().to_owned(),
            runtime,
            port,
            registries: Mutex::new(BTreeMap::new()),
            attempts: Mutex::new(Vec::new()),
            revocation: AtomicU64::new(0),
        })
    }
    pub(super) fn binding(&self, managed: &ManagedKeyScope) -> Result<Binding, AndroidError> {
        Binding::for_profile(
            &self.profile,
            managed.working_copy,
            managed.database.clone(),
            managed.root,
            managed.epoch,
        )
        .map_err(registry_error)
    }
    /// Catalogue equality is checked before constructing a registered descriptor
    /// writer; no arbitrary UI path is opened for a biometric operation.
    pub(super) fn current(
        &self,
        copy: &WorkingCopyView,
    ) -> Result<(Binding, ManagedKeyScope, Arc<PlatformCipherWriter>), AndroidError> {
        let registered = self
            .runtime
            .working_copies()
            .map_err(crate::document::runtime)?
            .into_iter()
            .any(|known| {
                known.path == Path::new(&copy.path)
                    && known.database.as_str() == copy.database
                    && known.root.to_string() == copy.root
            });
        if !registered {
            return Err(refusal(BiometryFailure::Stale));
        }
        let writer = self
            .runtime
            .platform_cipher_writer(Path::new(&copy.path))
            .map_err(crate::document::runtime)?;
        let snapshot = writer.snapshot().map_err(crate::document::runtime)?;
        let managed = DatabaseService::archive_key_scope(
            &snapshot,
            writer.working_copy().map_err(crate::document::runtime)?,
        )
        .map_err(|error| crate::document::runtime(error.into()))?;
        if managed.database.as_str() != copy.database || managed.root.to_string() != copy.root {
            return Err(refusal(BiometryFailure::Stale));
        }
        Ok((self.binding(&managed)?, managed, writer))
    }
    pub(super) fn reserve(&self, binding: &Binding) -> Result<Arc<Reservation>, AndroidError> {
        let key = binding.copy_digest().map_err(registry_error)?;
        let slot = {
            let mut slots = self.registries.lock().map_err(crate::document::failure)?;
            if let Some(slot) = slots.get(&key) {
                Arc::clone(slot)
            } else {
                let slot = Arc::new(RegistrySlot {
                    registry: Mutex::new(
                        Registry::new(&self.profile, binding.clone()).map_err(registry_error)?,
                    ),
                    busy: AtomicU64::new(0),
                    next: AtomicU64::new(1),
                });
                slots.insert(key, Arc::clone(&slot));
                slot
            }
        };
        let ticket = slot
            .next
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |next| {
                next.checked_add(1)
            })
            .map_err(|_| refusal(BiometryFailure::Limit))?;
        if slot
            .busy
            .compare_exchange(0, ticket, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(refusal(BiometryFailure::Busy));
        }
        let owned = Arc::new(Reservation { slot, ticket });
        owned
            .slot()
            .registry
            .lock()
            .map_err(crate::document::failure)?
            .current_binding(binding.clone())
            .map_err(registry_error)?;
        Ok(owned)
    }
    pub(super) fn availability(&self) -> Result<BiometricAvailability, AndroidError> {
        self.port.availability()
    }
    pub(super) fn require_available(&self) -> Result<(), AndroidError> {
        if self.availability()? != BiometricAvailability::Available {
            return Err(refusal(BiometryFailure::Unavailable));
        }
        Ok(())
    }
    pub(super) fn revision(&self) -> u64 {
        self.revocation.load(Ordering::Acquire)
    }
    pub(super) fn track(
        &self,
        fence: &Arc<AttemptFence>,
        revision: u64,
    ) -> Result<(), AndroidError> {
        let mut attempts = self.attempts.lock().map_err(crate::document::failure)?;
        attempts.retain(|attempt| attempt.strong_count() != 0);
        attempts.push(Arc::downgrade(fence));
        drop(attempts);
        if self.revision() != revision {
            fence.retire();
        }
        fence.check()
    }
    pub(crate) fn retire_all(&self) {
        self.revocation.fetch_add(1, Ordering::AcqRel);
        let attempts = self
            .attempts
            .lock()
            .map(|attempts| {
                attempts
                    .iter()
                    .filter_map(Weak::upgrade)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for attempt in attempts {
            attempt.retire();
        }
    }
    pub(crate) fn retire_generation(&self, generation: u64) {
        let attempts = self
            .attempts
            .lock()
            .map(|attempts| {
                attempts
                    .iter()
                    .filter_map(Weak::upgrade)
                    .filter(|attempt| attempt.lease().scope.generation == generation)
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        for attempt in attempts {
            attempt.retire();
        }
    }
    /// Revalidate exact OS lease and canonical RSA policy before worker wrap,
    /// final Enabled, or keyed boot. No OS callback runs under a registry mutex.
    pub(super) fn validate_key(
        &self,
        lease: &OsBiometricKeyLease,
        binding: &Binding,
        attempt: Digest,
    ) -> Result<(Generation, Digest), AndroidError> {
        let id = crate::document::digest(&lease.id)?;
        if id.to_string() != lease.id {
            return Err(refusal(BiometryFailure::Stale));
        }
        if lease.binding != BiometricBindingView::from_binding(binding)?
            || lease.attempt != attempt.to_string()
        {
            return Err(refusal(BiometryFailure::Stale));
        }
        let generation = Generation::parse(&lease.generation).map_err(registry_error)?;
        if lease.alias
            != WrapperRecord::scoped_alias(binding, generation).map_err(registry_error)?
        {
            return Err(refusal(BiometryFailure::Stale));
        }
        let spki = taypeer_runtime::platform_worker::biometry::validate_biometric_spki(&lease.spki)
            .map_err(crate::document::runtime)?;
        Ok((generation, spki))
    }
    pub(super) fn resolve(
        &self,
        record: &WrapperRecord,
        attempt: Digest,
    ) -> Result<OsBiometricKeyLease, AndroidError> {
        let lease = self.port.resolve_key(
            BiometricBindingView::from_binding(record.binding())?,
            record.generation().to_string(),
            record.alias().to_owned(),
            record.spki_digest().to_string(),
            attempt.to_string(),
        )?;
        let validated = self.validate_key(&lease, record.binding(), attempt);
        match validated {
            Ok((generation, spki))
                if generation == record.generation() && spki == record.spki_digest() =>
            {
                Ok(lease)
            }
            Ok(_) => {
                self.port.retire_lease(lease.id);
                Err(refusal(BiometryFailure::Stale))
            }
            Err(error) => {
                self.port.retire_lease(lease.id);
                Err(error)
            }
        }
    }
    pub(super) fn finish_setup(
        &self,
        owned: &Reservation,
        state: SetupState,
    ) -> Result<SetupState, AndroidError> {
        owned.check()?;
        match state {
            SetupState::PendingEnable {
                operation,
                candidate,
                ..
            } => {
                let current = self.latest_binding(candidate.binding())?;
                if current != *candidate.binding() {
                    // Only the already-recorded worker proof is being disposed.
                    // Do not resolve/prompt an old key or claim it is current.
                    let mut registry = owned
                        .slot()
                        .registry
                        .lock()
                        .map_err(crate::document::failure)?;
                    owned.check()?;
                    registry.current_binding(current).map_err(registry_error)?;
                    return registry
                        .finish_earlier_epoch_enable(operation)
                        .map_err(registry_error);
                }
                let lease = self.resolve(&candidate, operation)?;
                self.port.retire_lease(lease.id);
                self.require_current(candidate.binding())?;
                let mut registry = owned
                    .slot()
                    .registry
                    .lock()
                    .map_err(crate::document::failure)?;
                owned.check()?;
                registry.finish_enable(operation).map_err(registry_error)
            }
            SetupState::PendingDisable { operation, active } => {
                self.port.delete_key(
                    BiometricBindingView::from_binding(active.binding())?,
                    active.generation().to_string(),
                    active.alias().to_owned(),
                )?;
                let mut registry = owned
                    .slot()
                    .registry
                    .lock()
                    .map_err(crate::document::failure)?;
                owned.check()?;
                registry
                    .finish_disable(active.generation(), operation)
                    .map_err(registry_error)
            }
            SetupState::Enabled(_) | SetupState::Disabled => Ok(state),
        }
    }
    pub(super) fn require_current(&self, binding: &Binding) -> Result<(), AndroidError> {
        if self.latest_binding(binding)? == *binding {
            Ok(())
        } else {
            Err(refusal(BiometryFailure::Stale))
        }
    }
    fn latest_binding(&self, binding: &Binding) -> Result<Binding, AndroidError> {
        for copy in self
            .runtime
            .working_copies()
            .map_err(crate::document::runtime)?
        {
            if &copy.database != binding.database() || copy.root != binding.root() {
                continue;
            }
            let (_, managed, _) = self.current(&copy.into())?;
            if managed.working_copy == binding.working_copy() {
                return self.binding(&managed);
            }
        }
        Err(refusal(BiometryFailure::Stale))
    }
}
#[cfg(test)]
mod tests;
pub(super) fn random_digest() -> Result<Digest, AndroidError> {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| AndroidError::RandomUnavailable)?;
    Ok(Digest::from_bytes(bytes))
}
