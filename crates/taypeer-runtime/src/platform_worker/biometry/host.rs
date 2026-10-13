//! Rust-only Native adapter calls; there is no public UniFFI arbitrary-SPKI wrap getter.
use super::*;
use crate::Worker;

impl Worker {
    /// Read current password enrollment scope for the fixed trusted Native OS port.
    /// A biometric-open worker, stale projection or ordinary unconfigured worker refuses it.
    pub fn platform_biometric_scope(&mut self) -> Result<ManagedKeyScope, RuntimeError> {
        let value = self.client.request(
            &Envelope {
                biometry: Request::EnrollmentScope,
            },
            false,
        )?;
        serde_json::from_value(value).map_err(|_| RuntimeError::Protocol)
    }

    /// Private platform enrollment of the fixed OS owner's verified key.
    /// Native must not expose this DER parameter through a public UI/session wrap API.
    /// Only the separately configured private input owner can arm the exact lease.
    pub fn platform_prepare_biometric_enrollment(
        &mut self,
        lease: BiometricInputLease,
        spki: Vec<u8>,
    ) -> Result<BiometricWrappedKey, RuntimeError> {
        self.require_platform_biometric_lease(&lease)?;
        if spki.len() > 4096 {
            return Err(RuntimeError::Protocol);
        }
        let value = self.client.request(
            &Envelope {
                biometry: Request::PrepareEnrollment {
                    lease: lease.clone(),
                    spki,
                },
            },
            false,
        )?;
        let wrapped: BiometricWrappedKey =
            serde_json::from_value(value).map_err(|_| RuntimeError::Protocol)?;
        if wrapped.lease != lease || wrapped.ciphertext.len() != 256 {
            return Err(RuntimeError::Protocol);
        }
        Ok(wrapped)
    }

    /// Consume the already OS-completed private input and obtain an actual worker proof.
    /// OS prompt waits occur outside this call and outside the Native worker mutex.
    pub fn platform_confirm_biometric_enrollment(
        &mut self,
        lease: BiometricInputLease,
    ) -> Result<BiometricEnrollmentProof, RuntimeError> {
        self.require_platform_biometric_lease(&lease)?;
        let value = self.client.request(
            &Envelope {
                biometry: Request::ConfirmEnrollment(lease.clone()),
            },
            false,
        )?;
        let actual: BiometricInputLease =
            serde_json::from_value(value).map_err(|_| RuntimeError::Protocol)?;
        if actual != lease {
            return Err(RuntimeError::Protocol);
        }
        Ok(BiometricEnrollmentProof { lease })
    }

    /// Clear the current private attempt after the independent Native fence was retired.
    /// Lifecycle revocation itself must never wait for this command call.
    pub fn platform_cancel_biometric_enrollment(
        &mut self,
        lease: BiometricInputLease,
    ) -> Result<(), RuntimeError> {
        self.require_platform_biometric_lease(&lease)?;
        self.client.request(
            &Envelope {
                biometry: Request::Cancel(lease),
            },
            false,
        )?;
        Ok(())
    }

    fn require_platform_biometric_lease(
        &self,
        lease: &BiometricInputLease,
    ) -> Result<(), RuntimeError> {
        lease.require(BiometricInputPurpose::EnrollmentProof)?;
        let actual = self.control().status();
        if lease.scope.generation != actual.generation
            || &lease.scope.managed.database != self.database_id()
        {
            return Err(RuntimeError::Protocol);
        }
        Ok(())
    }
}
