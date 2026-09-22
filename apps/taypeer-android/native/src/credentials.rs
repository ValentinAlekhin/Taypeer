//! Keystore callbacks belong to the host, never the isolated document process.
use crate::AndroidError;
use std::{path::Path, sync::Arc};
use taypeer_runtime::{
    RuntimeHost,
    profile::{CredentialStore, ProfileError},
    session::{SessionController, SessionPolicy},
};
use zeroize::Zeroizing;

/// Android-protected bytes, addressed by opaque nonsecret service/account names.
/// Implementations authenticate the address as associated data and write durably.
#[uniffi::export(callback_interface)]
pub trait CredentialPort: Send + Sync {
    /// Missing differs from unavailable/corrupt. Never return an empty fallback on failure.
    fn get(&self, service: String, account: String) -> Result<Option<Vec<u8>>, AndroidError>;
    /// Return only after the encrypted value is durable. Erase the received byte array.
    fn set(&self, service: String, account: String, bytes: Vec<u8>) -> Result<(), AndroidError>;
}
struct Credentials(Box<dyn CredentialPort>);
impl CredentialStore for Credentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        self.0
            .get(service.to_owned(), account.to_owned())
            .map(|value| value.map(Zeroizing::new))
            .map_err(|_| ProfileError::Credentials)
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        self.0
            .set(service.to_owned(), account.to_owned(), bytes.to_vec())
            .map_err(|_| ProfileError::Credentials)
    }
}
/// Unique process-owned coordinator. Kotlin Application retains it, independently
/// of Activity recreation and document service lifetimes. No database is unlocked here.
#[derive(uniffi::Object)]
pub struct Host {
    runtime: RuntimeHost,
}
#[uniffi::export]
impl Host {
    /// Create one host using the application's private profile directory.
    /// Off the main thread; a second concurrent owner fails with a sanitized error.
    #[uniffi::constructor]
    pub fn new(
        directory: String,
        credentials: Box<dyn CredentialPort>,
    ) -> Result<Arc<Self>, AndroidError> {
        let sessions = SessionController::new(SessionPolicy::default());
        let runtime = RuntimeHost::with_platform_credentials(
            Path::new(&directory),
            sessions,
            Arc::new(Credentials(credentials)),
        )
        .map_err(|error| match error {
            taypeer_runtime::RuntimeError::Profile(ProfileError::Busy) => AndroidError::ProfileBusy,
            taypeer_runtime::RuntimeError::Profile(ProfileError::Io) => AndroidError::ProfileIo,
            taypeer_runtime::RuntimeError::Profile(ProfileError::Invalid) => {
                AndroidError::ProfileInvalid
            }
            taypeer_runtime::RuntimeError::Profile(ProfileError::Credentials) => {
                AndroidError::Credentials
            }
            _ => AndroidError::Runtime,
        })?;
        Ok(Arc::new(Self { runtime }))
    }
    /// Revoke all document generations on background entry without waiting for I/O.
    pub fn background(&self) {
        self.runtime
            .sessions()
            .lock_all(taypeer_runtime::session::LockReason::Background);
    }
    /// Record genuine foreground input; background exchange must never call this.
    pub fn activity(&self) {
        self.runtime.sessions().activity().touch();
    }
}
