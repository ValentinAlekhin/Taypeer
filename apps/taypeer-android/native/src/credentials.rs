//! Keystore callbacks belong to the host, never the isolated document process.
use crate::AndroidError;
use std::{
    path::Path,
    sync::{Arc, Mutex},
};
use taypeer_runtime::{
    RuntimeHost,
    profile::{CredentialStore, ProfileError},
    session::{SessionController, SessionSettings},
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
            .map_err(credential_error)
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        self.0
            .set(service.to_owned(), account.to_owned(), bytes.to_vec())
            .map_err(credential_error)
    }
}
fn credential_error(error: AndroidError) -> ProfileError {
    match error {
        AndroidError::CommitUncertain => ProfileError::CommitUncertain,
        AndroidError::ProfileInvalid => ProfileError::Invalid,
        AndroidError::ProfileIo => ProfileError::Io,
        _ => ProfileError::Credentials,
    }
}
/// Unique process-owned coordinator. Kotlin Application retains it, independently
/// of Activity recreation and document service lifetimes. No database is unlocked here.
#[derive(uniffi::Object)]
pub struct Host {
    pub(crate) runtime: Arc<RuntimeHost>,
    pub(crate) preferences: Mutex<crate::preferences::DevicePreferences>,
    pub(crate) biometry: Option<Arc<crate::biometry::Broker>>,
}
#[uniffi::export]
impl Host {
    /// Open the OS-independent encrypted profile. No credential-store callback or
    /// database transport secret is acquired before that database is authenticated.
    #[uniffi::constructor]
    pub fn portable(directory: String) -> Result<Arc<Self>, AndroidError> {
        Self::initialize_portable(directory, None)
    }
    /// Keep optional biometric key access separate from ordinary local credentials.
    #[uniffi::constructor]
    pub fn portable_with_os_biometry(
        directory: String,
        os_port: Box<dyn crate::OsBiometryPort>,
    ) -> Result<Arc<Self>, AndroidError> {
        Self::initialize_portable(directory, Some(Arc::from(os_port)))
    }
    /// Create one host using the application's private profile directory.
    /// Off the main thread; a second concurrent owner fails with a sanitized error.
    #[uniffi::constructor]
    pub fn new(
        directory: String,
        credentials: Box<dyn CredentialPort>,
    ) -> Result<Arc<Self>, AndroidError> {
        Self::initialize(directory, credentials, None)
    }
    /// Establish the immutable trusted OS owner before any UI/session exists.
    /// There is no post-construction setter or per-attempt adapter replacement.
    #[uniffi::constructor]
    pub fn with_os_biometry(
        directory: String,
        credentials: Box<dyn CredentialPort>,
        os_port: Box<dyn crate::OsBiometryPort>,
    ) -> Result<Arc<Self>, AndroidError> {
        Self::initialize(directory, credentials, Some(Arc::from(os_port)))
    }
    /// Revoke all document generations on background entry without waiting for I/O.
    pub fn background(&self) {
        if let Some(broker) = &self.biometry {
            broker.retire_all();
        }
        self.runtime
            .sessions()
            .lock_all(taypeer_runtime::session::LockReason::Background);
    }
    /// Record genuine foreground input; background exchange must never call this.
    pub fn activity(&self) {
        self.runtime.sessions().activity().touch();
    }
}
impl Host {
    fn initialize_portable(
        directory: String,
        os_port: Option<Arc<dyn crate::OsBiometryPort>>,
    ) -> Result<Arc<Self>, AndroidError> {
        let directory = Path::new(&directory);
        let sessions =
            SessionController::new(SessionSettings::load(directory).map_err(host_error)?);
        let preferences = crate::preferences::DevicePreferences::load(directory)?;
        let runtime =
            Arc::new(RuntimeHost::with_sessions(directory, sessions).map_err(host_error)?);
        let biometry = os_port.map(|port| crate::biometry::Broker::new(Arc::clone(&runtime), port));
        Ok(Arc::new(Self {
            runtime,
            preferences: Mutex::new(preferences),
            biometry,
        }))
    }
    fn initialize(
        directory: String,
        credentials: Box<dyn CredentialPort>,
        os_port: Option<Arc<dyn crate::OsBiometryPort>>,
    ) -> Result<Arc<Self>, AndroidError> {
        let directory = Path::new(&directory);
        let sessions =
            SessionController::new(SessionSettings::load(directory).map_err(host_error)?);
        let preferences = crate::preferences::DevicePreferences::load(directory)?;
        let runtime = RuntimeHost::with_platform_credentials(
            directory,
            sessions,
            Arc::new(Credentials(credentials)),
        )
        .map_err(host_error)?;
        let runtime = Arc::new(runtime);
        let biometry = os_port.map(|port| crate::biometry::Broker::new(Arc::clone(&runtime), port));
        Ok(Arc::new(Self {
            runtime,
            preferences: Mutex::new(preferences),
            biometry,
        }))
    }
}

pub(crate) fn host_error(error: taypeer_runtime::RuntimeError) -> AndroidError {
    match error {
        taypeer_runtime::RuntimeError::Profile(ProfileError::Busy) => AndroidError::ProfileBusy,
        taypeer_runtime::RuntimeError::Profile(ProfileError::Io) => AndroidError::ProfileIo,
        taypeer_runtime::RuntimeError::Profile(ProfileError::Invalid) => {
            AndroidError::ProfileInvalid
        }
        taypeer_runtime::RuntimeError::Profile(ProfileError::Credentials) => {
            AndroidError::Credentials
        }
        taypeer_runtime::RuntimeError::Profile(ProfileError::CommitUncertain)
        | taypeer_runtime::RuntimeError::Service(taypeer_services::ServiceError::Storage(
            taypeer_storage::Error::CommitUncertain,
        )) => AndroidError::CommitUncertain,
        _ => AndroidError::Runtime,
    }
}

#[cfg(test)]
mod tests;
