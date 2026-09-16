//! Nonsensitive device settings and recent file identities, separate from appearance settings.
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::{Path, PathBuf},
};
use taypeer_runtime::{RuntimeError, profile::ProfileError};

#[derive(Clone, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
/// Device-local configuration; contains no passwords, keys, or database plaintext.
#[non_exhaustive]
pub struct LocalSettings {
    /// Relay policy used when starting the network host.
    pub relay: RelayPreference,
    /// User-visible name announced for this device.
    pub device_name: String,
    /// Timeout for owned clipboard writes; None disables timed clearing.
    pub clipboard_seconds: Option<u32>,
    /// Recent file identities, newest first, bounded to twenty.
    pub recent: Vec<RecentFile>,
}
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
/// A public database identity paired with its last known local path.
#[non_exhaustive]
pub struct RecentFile {
    /// Stable database identity read from the public envelope.
    pub database: taypeer_core::DatabaseId,
    /// Last known local filesystem location.
    pub path: PathBuf,
}
impl Default for LocalSettings {
    fn default() -> Self {
        Self {
            relay: RelayPreference::Public,
            device_name: "Taypeer".into(),
            clipboard_seconds: Some(30),
            recent: Vec::new(),
        }
    }
}
impl LocalSettings {
    /// Load and validate device settings, defaulting only when the file is absent.
    pub fn load(profile: &Path) -> Result<Self, RuntimeError> {
        let file = match std::fs::File::open(profile.join("device-ui.json")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => return Err(ProfileError::Io.into()),
        };
        let mut bytes = Vec::new();
        file.take(65537)
            .read_to_end(&mut bytes)
            .map_err(|_| ProfileError::Io)?;
        if bytes.len() > 65536 {
            return Err(ProfileError::Invalid.into());
        }
        let value: Self = serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
        value.validate()?;
        Ok(value)
    }
    fn validate(&self) -> Result<(), RuntimeError> {
        self.relay.setting()?;
        if self.device_name.trim().is_empty()
            || self.device_name.len() > 256
            || self.clipboard_seconds.is_some_and(|v| v == 0)
            || self.recent.len() > 20
        {
            return Err(ProfileError::Invalid.into());
        }
        Ok(())
    }
    /// Update the bounded recent list, deduplicating both identity and path.
    pub fn remember(&mut self, database: taypeer_core::DatabaseId, path: PathBuf) {
        self.recent
            .retain(|r| r.database != database && r.path != path);
        self.recent.insert(0, RecentFile { database, path });
        self.recent.truncate(20);
    }
    /// Atomically save validated settings and sync the file and directory.
    pub fn save(&self, profile: &Path) -> Result<(), RuntimeError> {
        self.validate()?;
        Self::load(profile)?; // Preserve an unsupported or damaged file for explicit recovery.
        std::fs::create_dir_all(profile).map_err(|_| ProfileError::Io)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(profile).map_err(|_| ProfileError::Io)?;
        temporary
            .write_all(&serde_json::to_vec(self).map_err(|_| ProfileError::Invalid)?)
            .map_err(|_| ProfileError::Io)?;
        temporary
            .as_file()
            .sync_all()
            .map_err(|_| ProfileError::Io)?;
        temporary
            .persist(profile.join("device-ui.json"))
            .map_err(|_| ProfileError::Io)?;
        std::fs::File::open(profile)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| ProfileError::Io)?;
        Ok(())
    }
}

/// Product relay options exclude test-only certificate overrides and forced transports.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "url",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RelayPreference {
    #[default]
    /// Use the product public relay policy.
    Public,
    /// Disable relay use.
    Disabled,
    /// Use an explicitly configured HTTPS relay URL.
    Custom(String),
}
impl RelayPreference {
    /// Validate and convert the product preference into a transport setting.
    pub fn setting(&self) -> Result<taypeer_sync::RelaySetting, RuntimeError> {
        Ok(match self {
            Self::Public => taypeer_sync::RelaySetting::Default,
            Self::Disabled => taypeer_sync::RelaySetting::Disabled,
            Self::Custom(value) => {
                if !value.starts_with("https://") || value.len() > 2048 {
                    return Err(ProfileError::Invalid.into());
                }
                taypeer_sync::RelaySetting::Custom(
                    value.parse().map_err(|_| ProfileError::Invalid)?,
                )
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_settings_default_to_public_relay_and_custom_choice_survives_restart() {
        let directory = tempfile::tempdir().unwrap();
        std::fs::write(
            directory.path().join("device-ui.json"),
            br#"{"device_name":"PUBLIC old Mac","clipboard_seconds":17,"recent":[]}"#,
        )
        .unwrap();
        let mut settings = LocalSettings::load(directory.path()).unwrap();
        assert!(settings.relay == RelayPreference::Public);
        settings.relay = RelayPreference::Custom("https://relay.example.test".into());
        settings.save(directory.path()).unwrap();
        let restored = LocalSettings::load(directory.path()).unwrap();
        assert!(restored.relay == settings.relay);
        assert_eq!(restored.device_name, "PUBLIC old Mac");
        settings.relay = RelayPreference::Custom("http://relay.example.test".into());
        assert!(settings.save(directory.path()).is_err());
        assert!(LocalSettings::load(directory.path()).unwrap().relay == restored.relay);
    }
    #[test]
    fn settings_and_recent_paths_survive_restart_without_credentials() {
        let directory = tempfile::tempdir().unwrap();
        let mut settings = LocalSettings::load(directory.path()).unwrap();
        settings.device_name = "PUBLIC Mac / Мак".into();
        settings.clipboard_seconds = Some(17);
        settings.remember(
            taypeer_core::DatabaseId::new("PUBLIC db"),
            "PUBLIC-old.taypeer".into(),
        );
        settings.remember(
            taypeer_core::DatabaseId::new("PUBLIC db"),
            "PUBLIC-current.taypeer".into(),
        );
        settings.save(directory.path()).unwrap();
        let loaded = LocalSettings::load(directory.path()).unwrap();
        assert_eq!(loaded.device_name, settings.device_name);
        assert_eq!(loaded.clipboard_seconds, Some(17));
        assert_eq!(loaded.recent.len(), 1);
        assert_eq!(
            loaded.recent[0].path,
            PathBuf::from("PUBLIC-current.taypeer")
        );
    }
    #[test]
    fn unknown_settings_are_preserved_instead_of_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("device-ui.json");
        let future = br#"{"future_setting":"PUBLIC preserve"}"#;
        std::fs::write(&path, future).unwrap();
        assert!(LocalSettings::load(directory.path()).is_err());
        assert!(LocalSettings::default().save(directory.path()).is_err());
        assert_eq!(std::fs::read(path).unwrap(), future);
    }
}
