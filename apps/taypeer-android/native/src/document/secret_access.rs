//! Accepted secret actions own a separate opaque, content-free local metadata permission.
use super::{DocumentSession, runtime, session::identifier};
use crate::{AndroidError, credentials::host_error};
use std::sync::Arc;
use taypeer_core::{AttributeId, DatabaseId, EntryId, RevisionId};
use taypeer_runtime::Command;

/// Explicit saved secret and permission for one accepted display/copy action.
/// Clear the value on lock; never log it. Merely receiving this response writes nothing.
#[derive(uniffi::Record)]
pub struct SecretAccessView {
    /// Short-lived value with the same JVM/FFI zeroization limitations as legacy reveal.
    pub value: String,
    /// Confirm only after the UI accepted this exact binding, side and secret epoch.
    pub grant: Arc<SecretAccessGrant>,
}
impl std::fmt::Debug for SecretAccessView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretAccessView([REDACTED])")
    }
}

/// Opaque device-local metadata permission. It retains no value or document capability.
#[derive(uniffi::Object)]
pub struct SecretAccessGrant {
    event: taypeer_runtime::SecretAccessGrant,
    // Preserve the unique host lease while a metadata-only retry remains outstanding.
    _host: Arc<taypeer_runtime::RuntimeHost>,
}
#[uniffi::export]
impl SecretAccessGrant {
    /// Run off the UI thread after display acceptance or real clipboard ACK.
    /// A locked document does not undo an accepted action. Failure is metadata-only;
    /// retry this object without revealing/copying again. Returns the latest durable UTC ms.
    pub fn confirm(&self) -> Result<i64, AndroidError> {
        self.event
            .confirm()
            .map_err(|error| host_error(error.into()))
    }
}
#[uniffi::export]
impl DocumentSession {
    /// Reveal an authorized saved password/protected attribute and issue one opaque event.
    /// The atomic worker check rejects ordinary attributes and unfinished new entries.
    /// Legacy reveal remains available without implicit access recording.
    pub fn reveal_for_access(
        &self,
        entry: String,
        revision: Option<String>,
        attribute: Option<String>,
    ) -> Result<SecretAccessView, AndroidError> {
        let entry = EntryId::new(identifier(entry)?);
        let mut value =
            zeroize::Zeroizing::new(self.command::<String>(Command::RevealSavedForAccess {
                entry: entry.clone(),
                revision: revision.map(identifier).transpose()?.map(RevisionId::new),
                attribute: attribute.map(identifier).transpose()?.map(AttributeId::new),
            })?);
        let database = DatabaseId::new(identifier(self.identity()?.0)?);
        let event = self
            .host
            .secret_access_grant(database, entry)
            .map_err(runtime)?;
        Ok(SecretAccessView {
            value: std::mem::take(&mut *value),
            grant: Arc::new(SecretAccessGrant {
                event,
                _host: Arc::clone(&self.host),
            }),
        })
    }
}
