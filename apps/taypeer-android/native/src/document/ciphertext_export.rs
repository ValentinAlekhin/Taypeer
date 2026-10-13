//! A ciphertext-only SAF attempt survives ordinary document locking, never cancellation.
use super::{SelectedOutput, WorkingCopyView, digest, failure, runtime, session::identifier};
use crate::{AndroidError, Host, credentials::host_error};
use std::{
    io::Write,
    path::PathBuf,
    sync::{Arc, Mutex},
};
use taypeer_core::DatabaseId;
use taypeer_runtime::{CiphertextExport, WorkingCopy};

/// Exact captured public binding, without a source path, document name or key.
#[derive(Clone, uniffi::Record)]
pub struct EncryptedDatabaseCopyIdentity {
    /// Logical database identity.
    pub database: String,
    /// Profile-protected identity of the registered source.
    pub working_copy: String,
    /// Pinned trust-lineage root.
    pub root: String,
    /// Exact ciphertext fingerprint.
    pub fingerprint: String,
    /// Physical archive length, within the common archive limit.
    pub bytes: u64,
}

enum ExportState {
    Ready(Arc<CiphertextExport>),
    Writing(Arc<dyn SelectedOutput>),
    Cancelled,
}

/// One immutable encrypted archive capture and at most one selected destination.
/// It owns no worker or secret capability. Every provider callback runs without
/// the ownership mutex; cancelling never waits for an export callback to return.
#[derive(uniffi::Object)]
pub struct EncryptedDatabaseCopy {
    identity: EncryptedDatabaseCopyIdentity,
    state: Mutex<ExportState>,
}
impl EncryptedDatabaseCopy {
    fn new(capture: CiphertextExport) -> Self {
        let id = capture.identity();
        Self {
            identity: EncryptedDatabaseCopyIdentity {
                database: id.database.to_string(),
                working_copy: id.working_copy.to_string(),
                root: id.root.to_string(),
                fingerprint: id.fingerprint.to_string(),
                bytes: id.bytes,
            },
            state: Mutex::new(ExportState::Ready(Arc::new(capture))),
        }
    }

    fn begin(
        &self,
        output: &Arc<dyn SelectedOutput>,
    ) -> Result<Arc<CiphertextExport>, AndroidError> {
        let mut state = self.state.lock().map_err(failure)?;
        let capture = match &*state {
            ExportState::Ready(capture) => Arc::clone(capture),
            ExportState::Writing(_) => return Err(AndroidError::StorageBusy),
            ExportState::Cancelled => return Err(AndroidError::ExportCancelled),
        };
        *state = ExportState::Writing(Arc::clone(output));
        Ok(capture)
    }

    fn active(&self, output: &Arc<dyn SelectedOutput>) -> Result<(), AndroidError> {
        let state = self.state.lock().map_err(failure)?;
        match &*state {
            ExportState::Writing(current) if Arc::ptr_eq(current, output) => Ok(()),
            ExportState::Cancelled => Err(AndroidError::ExportCancelled),
            _ => Err(AndroidError::Runtime),
        }
    }

    fn copy(
        &self,
        capture: &CiphertextExport,
        output: &Arc<dyn SelectedOutput>,
    ) -> Result<(), AndroidError> {
        let mut writer = SelectedCiphertextOutput {
            owner: self,
            output,
            offset: 0,
            error: None,
        };
        if let Err(error) = capture.copy_to(&mut writer) {
            return Err(writer.error.unwrap_or_else(|| runtime(error)));
        }
        self.active(output)?;
        output.finish()?;
        self.active(output)
    }

    fn complete(
        &self,
        capture: Arc<CiphertextExport>,
        output: &Arc<dyn SelectedOutput>,
        result: Result<(), AndroidError>,
    ) -> Result<(), AndroidError> {
        let mut state = self.state.lock().map_err(failure)?;
        match &*state {
            ExportState::Writing(current) if Arc::ptr_eq(current, output) => {
                *state = ExportState::Ready(capture);
                result
            }
            // Cancellation between finish and completion cannot become an ACK.
            ExportState::Cancelled => Err(AndroidError::ExportCancelled),
            _ => Err(AndroidError::Runtime),
        }
    }
}

#[uniffi::export]
impl Host {
    /// Capture the exact registered encrypted file before launching the SAF picker.
    /// Locked sources are eligible; arbitrary paths/root substitutions are refused.
    pub fn capture_database_copy(
        &self,
        source: WorkingCopyView,
    ) -> Result<Arc<EncryptedDatabaseCopy>, AndroidError> {
        let source = WorkingCopy {
            database: DatabaseId::new(identifier(source.database)?),
            root: digest(&source.root)?,
            path: PathBuf::from(source.path),
        };
        self.runtime
            .capture_working_copy_export(&source)
            .map(|capture| Arc::new(EncryptedDatabaseCopy::new(capture)))
            .map_err(|error| match error {
                taypeer_runtime::RuntimeError::Profile(_) => host_error(error),
                _ => runtime(error),
            })
    }
}

#[uniffi::export]
impl EncryptedDatabaseCopy {
    /// Public identity remains readable after cancellation, without resource authority.
    pub fn identity(&self) -> EncryptedDatabaseCopyIdentity {
        self.identity.clone()
    }

    /// Stream the frozen ciphertext to one explicitly selected output off the UI thread.
    /// Success follows fingerprint verification and the output's durability/final-close
    /// acknowledgement. Failure permits an explicit new output for this same capture;
    /// a cancelled owner is never revived. Typed provider errors are preserved.
    pub fn export(&self, output: Box<dyn SelectedOutput>) -> Result<(), AndroidError> {
        let lease = OutputLease(Arc::from(output));
        let output = Arc::clone(&lease.0);
        let capture = self.begin(&output)?;
        let result = self.copy(&capture, &output);
        // Cleanup is never an ACK and runs before another destination is admitted.
        drop(lease);
        self.complete(capture, &output, result)
    }

    /// Irreversibly revoke this attempt and close its active output outside the mutex.
    /// An already written external file is not rolled back or silently deleted.
    pub fn cancel(&self) {
        let previous = match self.state.lock() {
            Ok(mut state) => std::mem::replace(&mut *state, ExportState::Cancelled),
            Err(_) => return,
        };
        if let ExportState::Writing(output) = previous {
            output.close();
        }
    }
}

struct OutputLease(Arc<dyn SelectedOutput>);
impl Drop for OutputLease {
    fn drop(&mut self) {
        self.0.close();
    }
}

struct SelectedCiphertextOutput<'a> {
    owner: &'a EncryptedDatabaseCopy,
    output: &'a Arc<dyn SelectedOutput>,
    offset: u64,
    error: Option<AndroidError>,
}
impl SelectedCiphertextOutput<'_> {
    fn write_chunk(&mut self, bytes: &[u8]) -> Result<usize, AndroidError> {
        self.owner.active(self.output)?;
        let count = bytes.len().min(crate::descriptors::TRANSFER_CHUNK);
        let written = self.output.write(self.offset, bytes[..count].to_vec())? as usize;
        if written > count {
            return Err(AndroidError::InvalidFile);
        }
        self.owner.active(self.output)?;
        self.offset = self
            .offset
            .checked_add(written as u64)
            .ok_or(AndroidError::InvalidFile)?;
        Ok(written)
    }
}
impl Write for SelectedCiphertextOutput<'_> {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        match self.write_chunk(bytes) {
            Ok(written) => Ok(written),
            Err(error) => {
                self.error = Some(error);
                Err(std::io::ErrorKind::Other.into())
            }
        }
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

#[cfg(test)]
mod tests;
