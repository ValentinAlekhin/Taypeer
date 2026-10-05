//! Platform credentials and password-authenticated per-database Linux registrations.
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::Arc,
};
use taypeer_core::DatabaseId;
use taypeer_storage::{Anchor, AnchorStore, ArchiveSnapshot};
use taypeer_trust::{AuthorKey, Digest, Identity, PublicKey, TransportKey};
use zeroize::Zeroizing;

mod credentials;
mod linux;
use credentials::Credentials;
pub use linux::TransportCapability;

/// Platform-protected credential store, supplied by the application host.
/// Implementations must authenticate encrypted values, bind them to service/account,
/// distinguish missing from unavailable/corrupt, and acknowledge only durable writes.
/// No plaintext file fallback is permitted. Calls run off the UI/supervisor thread.
pub trait CredentialStore: Send + Sync {
    /// Read at most 64 KiB into an owned zeroizing buffer; missing is `None`.
    fn get(&self, service: &str, account: &str)
    -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError>;
    /// Durably replace at most 64 KiB; preserve the previous value on pre-commit failure.
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError>;
}

/// Sanitized profile/credential failures; keychain diagnostics never escape this boundary.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{self:?}")]
pub enum ProfileError {
    /// Native secure credential storage is unavailable or denied access.
    Credentials,
    /// Public configuration or a protected registration is malformed.
    Invalid,
    /// Profile filesystem operation failed.
    Io,
    /// Another running host already owns this profile's transport endpoint.
    Busy,
    /// A local atomic replacement occurred but durability could not be confirmed.
    CommitUncertain,
}
impl From<std::fs::TryLockError> for ProfileError {
    fn from(error: std::fs::TryLockError) -> Self {
        match error {
            std::fs::TryLockError::WouldBlock => Self::Busy,
            std::fs::TryLockError::Error(_) => Self::Io,
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct PublicProfile {
    version: u16,
    id: Digest,
    transport: PublicKey,
}

/// Nonsecret profile handle. Cloning it never acquires an author credential.
#[derive(Clone)]
pub struct NativeProfile {
    directory: PathBuf,
    public: PublicProfile,
    credentials: Credentials,
}
/// Exclusive host ownership. Worker processes load only `NativeProfile`.
pub struct ProfileLease {
    profile: NativeProfile,
    _lock: File,
}
impl ProfileLease {
    /// Public handle passed to workers and platform adapters.
    pub fn profile(&self) -> &NativeProfile {
        &self.profile
    }
}

/// Protected binding of one physical working copy to its authority root and local draft.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Registration {
    /// Logical database identity; independent of trust lineage.
    pub database: DatabaseId,
    /// Root trusted at explicit registration/invitation/recovery.
    pub root: Digest,
    /// Local identity never transported with the archive.
    pub working_copy: Digest,
}
impl NativeProfile {
    pub(crate) fn load_state<T: serde::de::DeserializeOwned>(
        &self,
        key: &str,
    ) -> Result<Option<T>, ProfileError> {
        self.credentials
            .get(&self.service(), &format!("state:{key}"))?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid))
            .transpose()
    }
    pub(crate) fn save_state(&self, key: &str, value: &impl Serialize) -> Result<(), ProfileError> {
        let bytes = Zeroizing::new(serde_json::to_vec(value).map_err(|_| ProfileError::Invalid)?);
        if bytes.len() > 64 * 1024 {
            return Err(ProfileError::Invalid);
        }
        self.credentials
            .set(&self.service(), &format!("state:{key}"), &bytes)
    }
    /// Open/create a native profile and reserve its endpoint for this host lifetime.
    /// Only the transport role is acquired here; author access belongs to unlocked workers.
    pub fn acquire(directory: &Path) -> Result<ProfileLease, ProfileError> {
        #[cfg(target_os = "linux")]
        {
            linux::acquire(directory)
        }
        #[cfg(not(target_os = "linux"))]
        Self::acquire_with(directory, Credentials::Native)
    }
    /// Acquire the host profile using an explicitly supplied platform credential store.
    /// This does not grant an isolated worker filesystem or credential-store access.
    pub fn acquire_platform(
        directory: &Path,
        store: Arc<dyn CredentialStore>,
    ) -> Result<ProfileLease, ProfileError> {
        Self::acquire_with(directory, Credentials::Platform(store))
    }
    /// Create an isolated profile for public synthetic UI fixtures only.
    #[cfg(feature = "ui-test-support")]
    pub fn acquire_test(directory: &Path) -> Result<ProfileLease, ProfileError> {
        Self::acquire_with(
            directory,
            Credentials::Fixture(directory.join("public-credentials")),
        )
    }
    fn acquire_with(
        directory: &Path,
        credentials: Credentials,
    ) -> Result<ProfileLease, ProfileError> {
        std::fs::create_dir_all(directory).map_err(|_| ProfileError::Io)?;
        let directory = directory.canonicalize().map_err(|_| ProfileError::Io)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(directory.join("host.lock"))
            .map_err(|_| ProfileError::Io)?;
        taypeer_storage::try_lock_exclusive(&lock).map_err(ProfileError::from)?;
        let path = directory.join("profile.json");
        let profile = if path.try_exists().map_err(|_| ProfileError::Io)? {
            Self::load_with(&directory, credentials.clone())?
        } else {
            let id = random_id()?;
            let transport = TransportKey::generate().map_err(|_| ProfileError::Credentials)?;
            let profile = Self {
                directory,
                credentials,
                public: PublicProfile {
                    version: 1,
                    id,
                    transport: transport.public(),
                },
            };
            profile.credentials.set(
                &profile.service(),
                "transport",
                transport.secret_seed().as_ref(),
            )?;
            write_public(&path, &profile.public)?;
            profile
        };
        // A changed public file cannot redirect an established native credential.
        if profile.transport()?.public() != profile.public.transport {
            return Err(ProfileError::Invalid);
        }
        Ok(ProfileLease {
            profile,
            _lock: lock,
        })
    }
    /// Load public configuration only. This is the worker-side operation before authentication.
    pub fn load(directory: &Path) -> Result<Self, ProfileError> {
        #[cfg(target_os = "linux")]
        {
            Self::load_with(directory, Credentials::LazyLinux)
        }
        #[cfg(not(target_os = "linux"))]
        Self::load_with(directory, Credentials::Native)
    }
    /// Load only an explicitly selected synthetic fixture profile.
    #[cfg(feature = "ui-test-support")]
    pub fn load_test(directory: &Path) -> Result<Self, ProfileError> {
        Self::load_with(
            directory,
            Credentials::Fixture(directory.join("public-credentials")),
        )
    }
    fn load_with(directory: &Path, credentials: Credentials) -> Result<Self, ProfileError> {
        let directory = directory.canonicalize().map_err(|_| ProfileError::Io)?;
        let file = File::open(directory.join("profile.json")).map_err(|_| ProfileError::Io)?;
        if file.metadata().map_err(|_| ProfileError::Io)?.len() > 4096 {
            return Err(ProfileError::Invalid);
        }
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| ProfileError::Io)?;
        let public: PublicProfile =
            serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
        if public.version != 1 {
            return Err(ProfileError::Invalid);
        }
        public
            .transport
            .validate()
            .map_err(|_| ProfileError::Invalid)?;
        Ok(Self {
            directory,
            public,
            credentials,
        })
    }
    /// Local profile directory; encrypted credentials and state never enter portable archives.
    pub fn directory(&self) -> &Path {
        &self.directory
    }
    /// Public endpoint key available while every database is locked.
    pub fn transport_public(&self) -> PublicKey {
        self.public.transport
    }
    /// Explicit test credential provenance, never inferred from files or failed native access.
    pub(crate) fn is_public_fixture(&self) -> bool {
        #[cfg(feature = "ui-test-support")]
        {
            matches!(self.credentials, Credentials::Fixture(_))
        }
        #[cfg(not(feature = "ui-test-support"))]
        {
            false
        }
    }
    /// Acquire only the endpoint/manifest credential in the coordinator process.
    pub fn transport(&self) -> Result<TransportKey, ProfileError> {
        let seed = read_seed(&self.credentials, &self.service(), "transport")?
            .ok_or(ProfileError::Credentials)?;
        Ok(TransportKey::from_seed(&seed))
    }
    /// Acquire/enroll the independent author role after password authentication or explicit
    /// create/join enrollment. The caller owns its lifetime and must never send it to the host.
    pub fn author(&self) -> Result<AuthorKey, ProfileError> {
        let author = match read_seed(&self.credentials, &self.service(), "author")? {
            Some(seed) => AuthorKey::from_seed(&seed),
            None => {
                let author = AuthorKey::generate().map_err(|_| ProfileError::Credentials)?;
                self.credentials
                    .set(&self.service(), "author", author.secret_seed().as_ref())?;
                author
            }
        };
        let identity = Identity::new(author.public(), self.public.transport)
            .map_err(|_| ProfileError::Invalid)?;
        let bytes = serde_json::to_vec(&identity).map_err(|_| ProfileError::Invalid)?;
        self.credentials.set(&self.service(), "identity", &bytes)?;
        Ok(author)
    }
    /// Read enrolled public roles without acquiring the author seed.
    pub fn identity(&self) -> Result<Option<Identity>, ProfileError> {
        let Some(bytes) = self.credentials.get(&self.service(), "identity")? else {
            return Ok(None);
        };
        let identity: Identity =
            serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
        identity.validate().map_err(|_| ProfileError::Invalid)?;
        if identity.transport != self.public.transport {
            return Err(ProfileError::Invalid);
        }
        Ok(Some(identity))
    }
    /// Find a protected registration for the exact canonical working path.
    pub fn registration(&self, path: &Path) -> Result<Option<Registration>, ProfileError> {
        let account = copy_account(path)?;
        self.credentials
            .get(&self.service(), &account)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid))
            .transpose()
    }
    /// Register a verified copied file. This grants no author admission; its current
    /// fingerprint and pinned root are protected locally before a writer is attached.
    pub fn register(
        &self,
        path: &Path,
        snapshot: &ArchiveSnapshot,
    ) -> Result<Registration, ProfileError> {
        if let Some(registration) = self.registration(path)? {
            if registration.root != snapshot.chain().root().map_err(|_| ProfileError::Invalid)?
                || registration.database != snapshot.chain().head().database
            {
                return Err(ProfileError::Invalid);
            }
            return Ok(registration);
        }
        let registration = Registration {
            database: snapshot.chain().head().database.clone(),
            root: snapshot.chain().root().map_err(|_| ProfileError::Invalid)?,
            working_copy: random_id()?,
        };
        self.anchor(&registration)
            .save(&Anchor {
                accepted: Some(snapshot.fingerprint()),
                prepared: None,
            })
            .map_err(|_| ProfileError::Credentials)?;
        self.save_registration(path, &registration)?;
        Ok(registration)
    }
    /// Establish a protected creation intent before the coordinator publishes a new file.
    pub fn prepare_registration(
        &self,
        path: &Path,
        database: DatabaseId,
        root: Digest,
    ) -> Result<Registration, ProfileError> {
        if let Some(prior) = self.registration(path)?
            && (path.try_exists().map_err(|_| ProfileError::Io)?
                || self
                    .anchor(&prior)
                    .load()
                    .map_err(|_| ProfileError::Credentials)?
                    .is_none_or(|a| a.accepted.is_some()))
        {
            return Err(ProfileError::Invalid);
        }
        // An interrupted pre-publication intent has no working file to replace.
        // Its orphaned marker cannot grant admission to the fresh genesis below.
        let registration = Registration {
            database,
            root,
            working_copy: random_id()?,
        };
        self.anchor(&registration)
            .save(&Anchor {
                accepted: None,
                prepared: None,
            })
            .map_err(|_| ProfileError::Credentials)?;
        self.save_registration(path, &registration)?;
        Ok(registration)
    }
    /// Protected per-copy anti-rollback marker, separate from portable content.
    pub fn anchor(&self, registration: &Registration) -> Arc<dyn AnchorStore> {
        Arc::new(NativeAnchor {
            credentials: self.credentials.clone(),
            service: self.service(),
            account: format!("anchor:{}", registration.working_copy),
        })
    }
    fn save_registration(
        &self,
        path: &Path,
        registration: &Registration,
    ) -> Result<(), ProfileError> {
        let bytes = serde_json::to_vec(registration).map_err(|_| ProfileError::Invalid)?;
        self.credentials
            .set(&self.service(), &copy_account(path)?, &bytes)
    }

    /// Bind a relocated file to the same protected anchor and local draft identity.
    /// This does not enroll an author or replace an unrelated registration.
    pub(crate) fn relocate_registration(
        &self,
        source: &Path,
        destination: &Path,
    ) -> Result<Registration, ProfileError> {
        let registration = self.registration(source)?.ok_or(ProfileError::Invalid)?;
        if let Some(prior) = self.registration(destination)?
            && (prior.database != registration.database
                || prior.root != registration.root
                || prior.working_copy != registration.working_copy)
        {
            return Err(ProfileError::Invalid);
        }
        self.save_registration(destination, &registration)?;
        Ok(registration)
    }
    fn service(&self) -> String {
        format!("org.taypeer.dev4.{}", self.public.id)
    }
}
struct NativeAnchor {
    credentials: Credentials,
    service: String,
    account: String,
}
impl AnchorStore for NativeAnchor {
    fn load(&self) -> Result<Option<Anchor>, taypeer_storage::Error> {
        let bytes = self
            .credentials
            .get(&self.service, &self.account)
            .map_err(|_| taypeer_storage::Error::Io)?
            .ok_or(taypeer_storage::Error::Changed)?;
        Ok(Some(
            serde_json::from_slice(&bytes).map_err(|_| taypeer_storage::Error::InvalidFile)?,
        ))
    }
    fn save(&self, anchor: &Anchor) -> Result<(), taypeer_storage::Error> {
        let bytes = serde_json::to_vec(anchor).map_err(|_| taypeer_storage::Error::InvalidFile)?;
        self.credentials
            .set(&self.service, &self.account, &bytes)
            .map_err(|_| taypeer_storage::Error::Io)
    }
}
fn read_seed(
    credentials: &Credentials,
    service: &str,
    account: &str,
) -> Result<Option<Zeroizing<[u8; 32]>>, ProfileError> {
    credentials
        .get(service, account)?
        .map(|bytes| {
            let seed: &[u8; 32] = bytes
                .as_slice()
                .try_into()
                .map_err(|_| ProfileError::Invalid)?;
            Ok(Zeroizing::new(*seed))
        })
        .transpose()
}
fn copy_account(path: &Path) -> Result<String, ProfileError> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let canonical = if path.try_exists().map_err(|_| ProfileError::Io)? {
        path.canonicalize().map_err(|_| ProfileError::Io)?
    } else {
        parent
            .canonicalize()
            .map_err(|_| ProfileError::Io)?
            .join(path.file_name().ok_or(ProfileError::Invalid)?)
    };
    use std::os::unix::ffi::OsStrExt;
    Ok(format!(
        "copy:{}",
        Digest::of(canonical.as_os_str().as_bytes())
    ))
}
fn random_id() -> Result<Digest, ProfileError> {
    use rand_core::{OsRng, RngCore};
    let mut bytes = [0; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| ProfileError::Credentials)?;
    Ok(Digest::from_bytes(bytes))
}
fn write_public(path: &Path, profile: &PublicProfile) -> Result<(), ProfileError> {
    let mut temp = tempfile::NamedTempFile::new_in(path.parent().ok_or(ProfileError::Invalid)?)
        .map_err(|_| ProfileError::Io)?;
    let bytes = serde_json::to_vec(profile).map_err(|_| ProfileError::Invalid)?;
    temp.write_all(&bytes).map_err(|_| ProfileError::Io)?;
    taypeer_storage::publish_file(temp, path, taypeer_storage::PublicationMode::Create).map_err(
        |error| match error {
            taypeer_storage::Error::CommitUncertain => ProfileError::CommitUncertain,
            _ => ProfileError::Io,
        },
    )
}
#[cfg(target_os = "macos")]
mod native {
    use super::*;
    use security_framework::passwords::{
        PasswordOptions, generic_password, set_generic_password_options,
    };
    fn options(service: &str, account: &str) -> PasswordOptions {
        let mut options = PasswordOptions::new_generic_password(service, account);
        options.set_access_synchronized(Some(false));
        options
    }
    pub fn get(service: &str, account: &str) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        match generic_password(options(service, account)) {
            Ok(bytes) if bytes.len() <= 64 * 1024 => Ok(Some(Zeroizing::new(bytes))),
            Ok(bytes) => {
                drop(Zeroizing::new(bytes));
                Err(ProfileError::Invalid)
            }
            Err(error) if error.code() == -25300 => Ok(None), // errSecItemNotFound, distinct from denied access.
            Err(_) => Err(ProfileError::Credentials),
        }
    }
    pub fn set(service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        set_generic_password_options(bytes, options(service, account))
            .map_err(|_| ProfileError::Credentials)
    }
}
#[cfg(not(any(target_os = "macos", target_os = "linux")))]
mod native {
    use super::*;
    pub fn get(_: &str, _: &str) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        Err(ProfileError::Credentials)
    }
    pub fn set(_: &str, _: &str, _: &[u8]) -> Result<(), ProfileError> {
        Err(ProfileError::Credentials)
    }
}
