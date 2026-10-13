//! Validated local display names. No name is placed in a portable file or network message.
use super::ProfileError;
use crate::RuntimeError;
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

const MAX_NAME_BYTES: usize = 256;
const MAX_SETTINGS_BYTES: u64 = 4096;
const SETTINGS_FILE: &str = "device-name.json";
const SETTINGS_LOCK: &str = "device-name.lock";

/// A content-free refusal of a submitted local display name.
#[derive(Clone, Copy, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{self:?}")]
pub enum DeviceNameError {
    /// At least one non-whitespace character is required.
    Required,
    /// The original UTF-8 encoding exceeds 256 bytes.
    TooLong,
}

/// A validated device-local label. Validation preserves the original spelling and
/// whitespace; it never normalizes or announces the name to other devices.
#[derive(Clone, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct LocalDeviceName(String);
impl LocalDeviceName {
    /// Check the common local-name rule without copying or changing the input.
    pub fn validate(value: &str) -> Result<(), DeviceNameError> {
        if value.trim().is_empty() {
            return Err(DeviceNameError::Required);
        }
        if value.len() > MAX_NAME_BYTES {
            return Err(DeviceNameError::TooLong);
        }
        Ok(())
    }
    /// Borrow the exact confirmed local label for presentation.
    pub fn as_str(&self) -> &str {
        &self.0
    }
    /// Transfer the exact label to a platform presentation record.
    pub fn into_string(self) -> String {
        self.0
    }
}
impl TryFrom<String> for LocalDeviceName {
    type Error = DeviceNameError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::validate(&value)?;
        Ok(Self(value))
    }
}
impl Default for LocalDeviceName {
    fn default() -> Self {
        Self("Taypeer".into())
    }
}
impl std::fmt::Debug for LocalDeviceName {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("LocalDeviceName([redacted])")
    }
}
impl<'de> Deserialize<'de> for LocalDeviceName {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        String::deserialize(deserializer)?
            .try_into()
            .map_err(serde::de::Error::custom)
    }
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredName {
    version: u16,
    name: LocalDeviceName,
}

/// Bounded local configuration, independent of credentials and database unlocking.
pub struct DeviceNameSettings;
impl DeviceNameSettings {
    /// Default only for an absent file. Unknown versions, fields or malformed names
    /// fail explicitly and leave the existing configuration untouched.
    pub fn load(profile: &Path) -> Result<LocalDeviceName, RuntimeError> {
        let file = match File::open(profile.join(SETTINGS_FILE)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(LocalDeviceName::default());
            }
            Err(_) => return Err(ProfileError::Io.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_SETTINGS_BYTES + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ProfileError::Io)?;
        if bytes.len() as u64 > MAX_SETTINGS_BYTES {
            return Err(ProfileError::Invalid.into());
        }
        let stored: StoredName =
            serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
        if stored.version != 1 {
            return Err(ProfileError::Invalid.into());
        }
        Ok(stored.name)
    }
    /// Publish a complete validated name under a separate local lock. Success follows
    /// file and directory synchronization; uncertain publication must be reconciled
    /// by an explicit retry, never treated as a confirmed change or rollback.
    pub fn save(profile: &Path, name: &LocalDeviceName) -> Result<(), RuntimeError> {
        std::fs::create_dir_all(profile).map_err(|_| ProfileError::Io)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(profile.join(SETTINGS_LOCK))
            .map_err(|_| ProfileError::Io)?;
        taypeer_storage::try_lock_exclusive(&lock).map_err(ProfileError::from)?;
        Self::load(profile)?;
        let bytes = serde_json::to_vec(&StoredName {
            version: 1,
            name: name.clone(),
        })
        .map_err(|_| ProfileError::Invalid)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(profile).map_err(|_| ProfileError::Io)?;
        temporary.write_all(&bytes).map_err(|_| ProfileError::Io)?;
        taypeer_storage::publish_file(
            temporary,
            &profile.join(SETTINGS_FILE),
            taypeer_storage::PublicationMode::Replace,
        )
        .map_err(|error| match error {
            taypeer_storage::Error::CommitUncertain => ProfileError::CommitUncertain.into(),
            _ => ProfileError::Io.into(),
        })
    }
}

#[cfg(test)]
mod tests;
