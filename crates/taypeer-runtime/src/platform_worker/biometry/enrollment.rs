//! Only a live password provenance may enroll; the retained attempt contains no ReadKey.
use super::*;
use std::sync::Arc;
use taypeer_services::SessionToken;

struct Pending {
    wrapped: BiometricWrappedKey,
    proved: bool,
}
pub(in crate::platform_worker) struct Enrollment {
    inputs: Arc<dyn BiometricInputs>,
    pending: Option<Pending>,
}
impl Enrollment {
    pub(in crate::platform_worker) fn new(inputs: Arc<dyn BiometricInputs>) -> Self {
        Self {
            inputs,
            pending: None,
        }
    }
    pub(in crate::platform_worker) fn dispatch(
        &mut self,
        service: &DatabaseService,
        session: &SessionToken,
        request: Request,
    ) -> Result<serde_json::Value, RuntimeError> {
        match request {
            Request::EnrollmentScope => {
                let (scope, _key) = service.password_authenticated_key(session)?;
                serde_json::to_value(scope).map_err(|_| RuntimeError::Protocol)
            }
            Request::PrepareEnrollment { lease, spki } => {
                let wrapped = self.prepare(service, session, lease, &spki)?;
                serde_json::to_value(wrapped).map_err(|_| RuntimeError::Protocol)
            }
            Request::ConfirmEnrollment(lease) => {
                self.confirm(service, session, &lease)?;
                serde_json::to_value(lease).map_err(|_| RuntimeError::Protocol)
            }
            Request::Cancel(lease) => {
                self.inputs.retire(&lease);
                if self
                    .pending
                    .as_ref()
                    .is_some_and(|pending| pending.wrapped.lease == lease)
                {
                    self.pending = None;
                }
                Ok(serde_json::Value::Null)
            }
            Request::CompleteBoot(_) => Err(RuntimeError::Protocol),
        }
    }
    fn prepare(
        &mut self,
        service: &DatabaseService,
        session: &SessionToken,
        lease: BiometricInputLease,
        spki: &[u8],
    ) -> Result<BiometricWrappedKey, RuntimeError> {
        lease.require(BiometricInputPurpose::EnrollmentProof)?;
        let (scope, key) = service.password_authenticated_key(session)?;
        require_scope(&scope, &lease)?;
        let spki_digest = Digest::of(spki);
        if let Some(pending) = &self.pending {
            if pending.wrapped.lease != lease || pending.wrapped.spki != spki_digest {
                return Err(taypeer_services::ServiceError::InvalidContext.into());
            }
            if !pending.proved {
                self.inputs.check(&lease)?;
            }
            return Ok(pending.wrapped.clone());
        }
        let ciphertext = wrapping::wrap(&key, spki)?;
        let (latest, _key) = service.password_authenticated_key(session)?;
        require_scope(&latest, &lease)?;
        if let Err(error) = self
            .inputs
            .arm(&lease)
            .and_then(|()| self.inputs.check(&lease))
        {
            self.inputs.retire(&lease);
            return Err(error);
        }
        let wrapped = BiometricWrappedKey {
            lease,
            spki: spki_digest,
            ciphertext,
        };
        self.pending = Some(Pending {
            wrapped: wrapped.clone(),
            proved: false,
        });
        Ok(wrapped)
    }
    fn confirm(
        &mut self,
        service: &DatabaseService,
        session: &SessionToken,
        lease: &BiometricInputLease,
    ) -> Result<(), RuntimeError> {
        lease.require(BiometricInputPurpose::EnrollmentProof)?;
        let pending = self.pending.as_ref().ok_or(RuntimeError::Protocol)?;
        if &pending.wrapped.lease != lease {
            return Err(RuntimeError::Protocol);
        }
        let (scope, _key) = service.password_authenticated_key(session)?;
        require_scope(&scope, lease)?;
        if pending.proved {
            return Ok(());
        }
        self.inputs.check(lease)?;
        let retire = RetireInput {
            inputs: self.inputs.as_ref(),
            lease,
        };
        let key = super::super::read_private_key_input(self.inputs.consume(lease)?)?;
        self.inputs.check(lease)?;
        service.verify_password_authenticated_read_key(session, &lease.scope.managed, &key)?;
        self.inputs.check(lease)?;
        drop(retire);
        self.pending.as_mut().ok_or(RuntimeError::Protocol)?.proved = true;
        Ok(())
    }
}
impl Drop for Enrollment {
    fn drop(&mut self) {
        if let Some(pending) = &self.pending {
            self.inputs.retire(&pending.wrapped.lease);
        }
    }
}
