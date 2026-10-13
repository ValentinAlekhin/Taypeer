//! Validated, nonsecret device policies. UI state never owns the applied values.

use crate::{AndroidError, Host, credentials::host_error};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    num::NonZeroU32,
    path::Path,
};
use taypeer_runtime::{
    profile::{
        ProfileError,
        device_name::{DeviceNameError, DeviceNameSettings, LocalDeviceName},
    },
    session::{SessionPolicy, SessionSettings},
};

const CLIPBOARD_FILE: &str = "clipboard-policy.json";
const CLIPBOARD_LOCK: &str = "clipboard-policy.lock";

#[derive(Clone, Copy, Serialize, Deserialize)]
#[serde(tag = "mode", rename_all = "snake_case", deny_unknown_fields)]
enum ClipboardPolicy {
    Timed { seconds: NonZeroU32 },
    Never {},
}
impl Default for ClipboardPolicy {
    fn default() -> Self {
        Self::Timed {
            seconds: NonZeroU32::new(30).expect("documented default is positive"),
        }
    }
}
impl ClipboardPolicy {
    fn from_seconds(seconds: Option<u32>) -> Result<Self, AndroidError> {
        match seconds {
            Some(seconds) => Ok(Self::Timed {
                seconds: NonZeroU32::new(seconds).ok_or(AndroidError::InvalidOptions)?,
            }),
            None => Ok(Self::Never {}),
        }
    }
    fn seconds(self) -> Option<u32> {
        match self {
            Self::Timed { seconds } => Some(seconds.get()),
            Self::Never {} => None,
        }
    }
}
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredClipboard {
    version: u16,
    clipboard: ClipboardPolicy,
}

pub(crate) struct DevicePreferences {
    clipboard: ClipboardPolicy,
    device_name: LocalDeviceName,
}
impl DevicePreferences {
    pub(crate) fn load(profile: &Path) -> Result<Self, AndroidError> {
        let device_name = DeviceNameSettings::load(profile).map_err(host_error)?;
        let clipboard = Self::load_clipboard(profile)?;
        Ok(Self {
            clipboard,
            device_name,
        })
    }
    fn load_clipboard(profile: &Path) -> Result<ClipboardPolicy, AndroidError> {
        let file = match File::open(profile.join(CLIPBOARD_FILE)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(ClipboardPolicy::default());
            }
            Err(_) => return Err(AndroidError::ProfileIo),
        };
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| AndroidError::ProfileIo)?;
        if bytes.len() > 4096 {
            return Err(AndroidError::ProfileInvalid);
        }
        let stored: StoredClipboard =
            serde_json::from_slice(&bytes).map_err(|_| AndroidError::ProfileInvalid)?;
        if stored.version != 1 {
            return Err(AndroidError::ProfileInvalid);
        }
        Ok(stored.clipboard)
    }
    fn save_clipboard(
        &mut self,
        profile: &Path,
        policy: ClipboardPolicy,
    ) -> Result<(), AndroidError> {
        std::fs::create_dir_all(profile).map_err(|_| AndroidError::ProfileIo)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(profile.join(CLIPBOARD_LOCK))
            .map_err(|_| AndroidError::ProfileIo)?;
        taypeer_storage::try_lock_exclusive(&lock)
            .map_err(ProfileError::from)
            .map_err(|error| host_error(error.into()))?;
        Self::load_clipboard(profile)?; // Preserve damaged or unsupported data for explicit recovery.
        let bytes = serde_json::to_vec(&StoredClipboard {
            version: 1,
            clipboard: policy,
        })
        .map_err(|_| AndroidError::ProfileInvalid)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(profile).map_err(|_| AndroidError::ProfileIo)?;
        temporary
            .write_all(&bytes)
            .map_err(|_| AndroidError::ProfileIo)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| AndroidError::ProfileIo)?;
        temporary
            .persist(profile.join(CLIPBOARD_FILE))
            .map_err(|_| AndroidError::ProfileIo)?;
        File::open(profile)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| AndroidError::ProfileIo)?;
        self.clipboard = policy;
        Ok(())
    }
    fn save_name(&mut self, profile: &Path, name: LocalDeviceName) -> Result<(), AndroidError> {
        self.confirm_name(name, |candidate| {
            DeviceNameSettings::save(profile, candidate).map_err(host_error)
        })
    }
    fn confirm_name(
        &mut self,
        name: LocalDeviceName,
        publish: impl FnOnce(&LocalDeviceName) -> Result<(), AndroidError>,
    ) -> Result<(), AndroidError> {
        publish(&name)?;
        self.device_name = name;
        Ok(())
    }
    fn view(&self, policy: SessionPolicy) -> DeviceSettingsView {
        DeviceSettingsView {
            idle_seconds: policy.idle_seconds(),
            clipboard_seconds: self.clipboard.seconds(),
            device_name: self.device_name.as_str().to_owned(),
        }
    }
}

/// Applied nonsecret device policies. None disables Taypeer's clipboard timer.
#[derive(PartialEq, Eq, uniffi::Record)]
pub struct DeviceSettingsView {
    /// Positive idle interval for the common session supervisor.
    pub idle_seconds: u32,
    /// Positive owned-clipboard interval, or no Taypeer clearing timer.
    pub clipboard_seconds: Option<u32>,
    /// Exact acknowledged label for this device only; not a peer/author name.
    pub device_name: String,
}
impl std::fmt::Debug for DeviceSettingsView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("DeviceSettingsView")
            .field("idle_seconds", &self.idle_seconds)
            .field("clipboard_seconds", &self.clipboard_seconds)
            .field("device_name", &"[redacted]")
            .finish()
    }
}

#[uniffi::export]
impl Host {
    /// Read applied policies. Corrupt preferences fail host creation, never default silently.
    pub fn device_settings(&self) -> Result<DeviceSettingsView, AndroidError> {
        let preferences = self.preferences.lock().map_err(|_| AndroidError::Runtime)?;
        Ok(preferences.view(self.runtime.sessions().policy()))
    }
    /// Confirm a local display label only after durable publication. Names remain
    /// local; no network restart, database write or historical author rewrite occurs.
    /// Even the current cached value is republished so an uncertain prior write
    /// cannot leave an unconfirmed different value on disk across restart.
    pub fn set_device_name(&self, name: String) -> Result<DeviceSettingsView, AndroidError> {
        let name = LocalDeviceName::try_from(name).map_err(|error| match error {
            DeviceNameError::Required => AndroidError::DeviceNameRequired,
            DeviceNameError::TooLong => AndroidError::DeviceNameTooLong,
        })?;
        let mut preferences = self.preferences.lock().map_err(|_| AndroidError::Runtime)?;
        preferences.save_name(self.runtime.profile().directory(), name)?;
        Ok(preferences.view(self.runtime.sessions().policy()))
    }
    /// Validate and durably save before hot application. An I/O error after publication
    /// cannot confirm the on-disk policy; it never reports the new value as applied.
    pub fn set_idle_seconds(&self, seconds: u32) -> Result<DeviceSettingsView, AndroidError> {
        let policy = SessionPolicy::new(seconds).ok_or(AndroidError::InvalidOptions)?;
        let preferences = self.preferences.lock().map_err(|_| AndroidError::Runtime)?;
        let profile = self.runtime.profile().directory();
        SessionSettings::load(profile).map_err(host_error)?;
        SessionSettings::save(profile, policy).map_err(host_error)?;
        self.runtime.sessions().set_policy(policy);
        Ok(preferences.view(policy))
    }
    /// Validate and durably save a positive clipboard interval or explicit no-clean mode.
    /// Serialized with idle policy publication; contains no clipboard values or receipts.
    pub fn set_clipboard_seconds(
        &self,
        seconds: Option<u32>,
    ) -> Result<DeviceSettingsView, AndroidError> {
        let policy = ClipboardPolicy::from_seconds(seconds)?;
        let mut preferences = self.preferences.lock().map_err(|_| AndroidError::Runtime)?;
        preferences.save_clipboard(self.runtime.profile().directory(), policy)?;
        Ok(preferences.view(self.runtime.sessions().policy()))
    }
}

#[cfg(test)]
mod tests;
