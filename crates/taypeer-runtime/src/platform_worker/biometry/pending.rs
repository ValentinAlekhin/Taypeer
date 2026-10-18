//! A supervised Opening owner relinquishes custody only to an authenticated Worker.
use super::*;
use crate::{
    RuntimeHost, Worker, WorkerControl,
    host::HostContext,
    platform::ProcessLauncher,
    platform_worker::PlatformBoot,
    process::Client,
    session::{LockOutcome, LockReason, SessionController},
};
use std::sync::Arc;

/// Public binding chosen by the fixed Native owner before an isolated keyed launch.
/// This contains no key, FD, authentication flag or UI-selected process generation.
pub struct BiometricBootAttempt {
    /// Canonical private profile namespace of the registered local copy.
    pub namespace: Digest,
    /// Current immutable registered ciphertext/checkpoint projection.
    pub managed: ManagedKeyScope,
    /// Native-minted random identity of this attempt.
    pub attempt: Digest,
    /// Native-minted identity of the separate one-shot input slot.
    pub input: Digest,
}

/// Own a real supervised process which has armed its private input but has not opened.
/// No command, lifecycle mutex or actor request is held while Native runs the OS prompt.
/// Drop revokes unfinished custody; its independent control retains the real close outcome.
pub struct PendingBiometricOpen {
    client: Option<Arc<Client>>,
    challenge: BiometricInputLease,
    context: Arc<HostContext>,
    runtime: tokio::runtime::Handle,
    sessions: SessionController,
}
impl RuntimeHost {
    /// Complete the authenticated keyed boot and start any configured exchange.
    /// A pending launch belongs to this exact host and cannot activate another profile.
    pub fn complete_platform_biometric_open(
        &self,
        pending: PendingBiometricOpen,
        lease: &BiometricInputLease,
    ) -> Result<Worker, RuntimeError> {
        if !Arc::ptr_eq(&self.context, &pending.context) {
            return Err(RuntimeError::Protocol);
        }
        let worker = pending.complete(lease)?;
        self.start_requested_network();
        Ok(worker)
    }

    /// Launch the ordinary descriptor process and check its exact keyed Opening challenge.
    /// Supervision starts before boot or prompt; no author/session is acquired by this step.
    /// A refused/stale/unconfigured boot revokes the child before returning an error.
    pub fn begin_platform_biometric_open(
        &self,
        launcher: &dyn ProcessLauncher,
        attempt: BiometricBootAttempt,
    ) -> Result<PendingBiometricOpen, RuntimeError> {
        let client = Client::connect_platform(launcher.launch()?, &self.sessions)?;
        let challenge = BiometricInputLease {
            scope: BiometricAttemptScope {
                namespace: attempt.namespace,
                managed: attempt.managed,
                generation: client.control.status().generation,
                attempt: attempt.attempt,
            },
            input: attempt.input,
            purpose: BiometricInputPurpose::KeyedBoot,
        };
        let boot = PlatformBoot {
            password: String::new(),
            form: None,
            invitation: None,
            recovery: None,
            biometry: Some(challenge.clone()),
        };
        let actual = client.request(&boot, true).and_then(|value| {
            serde_json::from_value::<BiometricInputLease>(value).map_err(|_| RuntimeError::Protocol)
        });
        let checked = actual.and_then(|actual| {
            if actual == challenge {
                Ok(())
            } else {
                Err(RuntimeError::Protocol)
            }
        });
        if let Err(error) = checked {
            client.control.invalidate(LockReason::Transport);
            return Err(error);
        }
        Ok(PendingBiometricOpen {
            client: Some(client),
            challenge,
            context: Arc::clone(&self.context),
            runtime: self.runtime.handle().clone(),
            sessions: self.sessions.clone(),
        })
    }
}
impl PendingBiometricOpen {
    /// The exact armed scope, including the real supervised process generation.
    pub fn challenge(&self) -> &BiometricInputLease {
        &self.challenge
    }
    /// Independent cancellation/status handle, available before any OPEN session exists.
    pub fn control(&self) -> WorkerControl {
        WorkerControl(Arc::clone(&self.client().control))
    }
    /// Consume this owner only after Native has installed the exact OS-completed input.
    /// Author access follows current checkpoint authentication inside the child, never here.
    /// A stale/cancelled/error reply cannot become a Worker or revive a closed generation.
    fn complete(mut self, lease: &BiometricInputLease) -> Result<Worker, RuntimeError> {
        if lease != &self.challenge {
            return Err(RuntimeError::Protocol);
        }
        let client = Arc::clone(self.client());
        let database = client
            .request(
                &Envelope {
                    biometry: Request::CompleteBoot(lease.clone()),
                },
                true,
            )
            .and_then(|value| serde_json::from_value(value).map_err(|_| RuntimeError::Protocol))?;
        if database != self.challenge.scope.managed.database {
            return Err(RuntimeError::Protocol);
        }
        let worker = Worker::from_client(
            client,
            database,
            Arc::clone(&self.context),
            &self.runtime,
            &self.sessions,
        )?;
        self.client = None;
        Ok(worker)
    }
    /// Revoke Opening and await the existing bounded actual worker/OS shutdown contract.
    /// Native retires its independent FD/OS writer fence before invoking this wait off-main.
    pub fn close_report(&mut self) -> Result<LockOutcome, RuntimeError> {
        let client = self.client();
        client.control.invalidate(LockReason::Manual);
        client.control.wait_closed()
    }
    fn client(&self) -> &Arc<Client> {
        self.client
            .as_ref()
            .expect("pending custody exists until consumed")
    }
}
impl Drop for PendingBiometricOpen {
    fn drop(&mut self) {
        if let Some(client) = &self.client {
            client.control.invalidate(LockReason::HostExited);
        }
    }
}
