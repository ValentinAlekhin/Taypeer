//! Public status follows exact registry and actual OS generation checks.
use super::{
    BiometricAvailability, BiometricBindingView, BiometryFailure, refusal,
    registry::{Binding, Generation, SetupState},
    registry_error,
};
use crate::{AndroidError, Host, WorkingCopyView};

/// Local biometric setup state; Enabled never means the database is OPEN.
#[derive(Clone, Copy, Debug, PartialEq, Eq, uniffi::Enum)]
pub enum BiometryState {
    /// No confirmed local wrapper exists.
    Disabled,
    /// Exact current wrapper is durable and its OS generation resolves.
    Enabled,
    /// Previous wrapper cannot authenticate the current epoch or OS policy.
    NeedsMaster,
    /// Exact proof-confirmed setup/disable requires reconciliation.
    Pending,
}
/// Nonsecret UI actions for one exact registered local copy.
#[derive(uniffi::Record)]
pub struct BiometryStatusView {
    /// Durable setup/eligibility projection, distinct from access.
    pub state: BiometryState,
    /// Current permanent scope, never a key or human-readable alias.
    pub binding: BiometricBindingView,
    /// Exact wrapper generation for Disable selection.
    pub generation: Option<String>,
    /// Exact pending operation, absent after completed publication.
    pub operation: Option<String>,
    /// Actual strong-biometric availability.
    pub availability: BiometricAvailability,
}
impl super::Broker {
    pub(super) fn view(
        &self,
        binding: &Binding,
        state: Option<SetupState>,
    ) -> Result<BiometryStatusView, AndroidError> {
        let availability = self.availability()?;
        let mut view = BiometryStatusView {
            state: BiometryState::Disabled,
            binding: BiometricBindingView::from_binding(binding)?,
            generation: None,
            operation: None,
            availability,
        };
        match state {
            None | Some(SetupState::Disabled) => {}
            Some(SetupState::PendingEnable {
                operation,
                candidate,
                ..
            }) => {
                view.state = BiometryState::Pending;
                view.generation = Some(candidate.generation().to_string());
                view.operation = Some(operation.to_string());
            }
            Some(SetupState::PendingDisable { operation, active }) => {
                view.state = BiometryState::Pending;
                view.generation = Some(active.generation().to_string());
                view.operation = Some(operation.to_string());
            }
            Some(SetupState::Enabled(active)) => {
                view.generation = Some(active.generation().to_string());
                if active.binding() != binding || availability != BiometricAvailability::Available {
                    view.state = BiometryState::NeedsMaster;
                } else {
                    match self.resolve(&active, super::broker::random_digest()?) {
                        Ok(lease) => {
                            self.port.retire_lease(lease.id);
                            self.require_current(binding)?;
                            view.state = BiometryState::Enabled;
                        }
                        Err(AndroidError::Biometry {
                            failure: BiometryFailure::Unavailable,
                        }) => view.state = BiometryState::NeedsMaster,
                        Err(error) => return Err(error),
                    }
                }
            }
        }
        Ok(view)
    }
}
impl Host {
    pub(crate) fn biometric_broker(&self) -> Result<&std::sync::Arc<super::Broker>, AndroidError> {
        self.biometry
            .as_ref()
            .ok_or_else(|| refusal(BiometryFailure::Unavailable))
    }
}
#[uniffi::export]
impl Host {
    /// Read confirmed exact setup and resolve the current actual OS generation.
    /// Corrupt/future/unknown publication remains a typed error, never Disabled.
    pub fn biometry_status(
        &self,
        copy: WorkingCopyView,
    ) -> Result<BiometryStatusView, AndroidError> {
        let broker = self.biometric_broker()?;
        let (binding, _, _) = broker.current(&copy)?;
        let owned = broker.reserve(&binding)?;
        let state = owned
            .slot()
            .registry
            .lock()
            .map_err(crate::document::failure)?
            .confirmed()
            .map_err(registry_error)?;
        broker.view(&binding, state)
    }
    /// Reconcile only an existing exact proof-confirmed setup, without new wrap,
    /// key generation, FD or prompt. Equal disk bytes still require durable sync.
    pub fn retry_biometric_setup(
        &self,
        copy: WorkingCopyView,
        operation: String,
    ) -> Result<BiometryStatusView, AndroidError> {
        let broker = self.biometric_broker()?;
        let (binding, _, _) = broker.current(&copy)?;
        let owned = broker.reserve(&binding)?;
        let operation = crate::document::digest(&operation)?;
        let state = owned
            .slot()
            .registry
            .lock()
            .map_err(crate::document::failure)?
            .retry(operation)
            .map_err(registry_error)?;
        let state = broker.finish_setup(&owned, state)?;
        broker.view(&binding, Some(state))
    }
    /// Disable only the exact selected generation. A stale receipt cannot delete
    /// or overwrite a later reenrollment; unknown publication preserves retry.
    pub fn disable_biometry(
        &self,
        copy: WorkingCopyView,
        expected_generation: String,
        operation: String,
    ) -> Result<BiometryStatusView, AndroidError> {
        let broker = self.biometric_broker()?;
        let (binding, _, _) = broker.current(&copy)?;
        let owned = broker.reserve(&binding)?;
        let expected = Generation::parse(&expected_generation).map_err(registry_error)?;
        let operation = crate::document::digest(&operation)?;
        let state = owned
            .slot()
            .registry
            .lock()
            .map_err(crate::document::failure)?
            .begin_disable(expected, operation)
            .map_err(registry_error)?;
        let state = broker.finish_setup(&owned, state)?;
        broker.view(&binding, Some(state))
    }
}
