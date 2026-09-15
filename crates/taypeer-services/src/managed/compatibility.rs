use super::*;
use taypeer_core::CompatibilityReport;

pub(crate) fn require_read(report: &CompatibilityReport) -> Result<(), ServiceError> {
    if report.read.is_supported() {
        Ok(())
    } else {
        Err(ServiceError::ReadCompatibility)
    }
}
pub(crate) fn require_write(report: &CompatibilityReport) -> Result<(), ServiceError> {
    require_read(report)?;
    if report.write.is_supported() {
        Ok(())
    } else {
        Err(ServiceError::WriteCompatibility)
    }
}
impl ManagedState {
    pub(super) fn packet_compatibility(
        &self,
        object: &EncryptedObject,
        metadata: &Checkpoint,
    ) -> Result<CompatibilityReport, ServiceError> {
        let origin = metadata.origin_chain(self.snapshot.chain(), object.envelope().trust_set)?;
        Ok(self
            .capabilities
            .assess(&origin.at(object.envelope().control)?.schema))
    }
    pub(super) fn check_format_write(&self) -> Result<(), ServiceError> {
        let snapshot = self.port.snapshot()?;
        require_write(&self.capabilities.assess(&snapshot.chain().head().schema))
    }
}
impl DatabaseService {
    /// Authenticated format requirements, independent of the current profile's admission.
    /// The session remains responsible for lock state and every command's permissions.
    pub fn compatibility(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<CompatibilityReport>, ServiceError> {
        let state = self.checked(session)?;
        let schema = match &state.managed {
            Some(managed) => managed.port.snapshot()?.chain().head().schema.clone(),
            None => state.document().schema_descriptor()?,
        };
        Ok(stamped(session, self.capabilities.assess(&schema)))
    }
}
