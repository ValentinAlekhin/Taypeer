//! Device-local relay selection, durability and serialized network reconfiguration.
use super::{RelaySetting, RuntimeError, RuntimeHost};
use crate::profile::ProfileError;
use serde::{Deserialize, Serialize};
use std::{
    io::{Read, Write},
    path::Path,
};

const SETTINGS_FILE: &str = "network.json";
const MAX_SETTINGS: u64 = 4096;

/// Product relay choices. Private certificate overrides and forced relay paths
/// belong to explicit test capabilities, never a saved product preference.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "mode",
    content = "url",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum RelayPreference {
    /// Public relays and Iroh address lookup, with direct paths where reachable.
    #[default]
    Public,
    /// Direct paths only; relay and public address lookup are disabled.
    Disabled,
    /// An explicitly supplied HTTPS relay with ordinary certificate verification.
    Custom(String),
}
impl RelayPreference {
    /// Validate and map a product selection without starting network I/O.
    pub fn setting(&self) -> Result<RelaySetting, RuntimeError> {
        Ok(match self {
            Self::Public => RelaySetting::Default,
            Self::Disabled => RelaySetting::Disabled,
            Self::Custom(value) => {
                if value.len() > 2048 || !value.starts_with("https://") {
                    return Err(ProfileError::Invalid.into());
                }
                let url = value
                    .parse::<taypeer_sync::RelayUrl>()
                    .map_err(|_| ProfileError::Invalid)?;
                if url.host_str().is_none()
                    || !url.username().is_empty()
                    || url.password().is_some()
                {
                    return Err(ProfileError::Invalid.into());
                }
                RelaySetting::Custom(url)
            }
        })
    }
    fn load(directory: &Path) -> Result<Self, RuntimeError> {
        let file = match std::fs::File::open(directory.join(SETTINGS_FILE)) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(Self::default());
            }
            Err(_) => return Err(ProfileError::Io.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_SETTINGS + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ProfileError::Io)?;
        if bytes.len() as u64 > MAX_SETTINGS {
            return Err(ProfileError::Invalid.into());
        }
        let preference: Self = serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
        preference.setting()?;
        Ok(preference)
    }
    fn save(&self, directory: &Path) -> Result<(), RuntimeError> {
        self.setting()?;
        let mut file = tempfile::NamedTempFile::new_in(directory).map_err(|_| ProfileError::Io)?;
        file.write_all(&serde_json::to_vec(self).map_err(|_| ProfileError::Invalid)?)
            .map_err(|_| ProfileError::Io)?;
        taypeer_storage::publish_file(
            file,
            &directory.join(SETTINGS_FILE),
            taypeer_storage::PublicationMode::Replace,
        )
        .map_err(|error| match error {
            taypeer_storage::Error::CommitUncertain => ProfileError::CommitUncertain.into(),
            _ => ProfileError::Io.into(),
        })
    }
}
impl RuntimeHost {
    /// Read the bounded local preference. Only an absent file defaults to Public.
    pub fn relay_preference(&self) -> Result<RelayPreference, RuntimeError> {
        RelayPreference::load(self.profile().directory())
    }
    /// Start a network lifetime using the durable product preference.
    pub fn start_configured_network(&self) -> Result<(), RuntimeError> {
        let _configuration = self
            .network_configuration
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        self.start_network(self.relay_preference()?.setting()?)
            .map(|_| ())
    }
    /// Stop the configured lifetime without overlapping a preference transition.
    pub fn stop_configured_network(&self) {
        if let Ok(_configuration) = self.network_configuration.lock() {
            self.stop_network();
        }
    }
    /// Persist a validated choice only after a running host accepts it. On startup
    /// or publication failure restore the previous lifetime and choice. Uncertain
    /// durability is returned to the caller and never reported as success.
    pub fn set_relay_preference(&self, preference: RelayPreference) -> Result<(), RuntimeError> {
        let setting = preference.setting()?;
        let _configuration = self
            .network_configuration
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let previous = self.relay_preference()?;
        if preference == previous {
            return Ok(());
        }
        let running = self
            .requested_relay
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .clone();
        if running.is_some() {
            self.stop_network();
            if let Err(error) = self.start_network(setting) {
                self.restore_network(running.as_ref());
                return Err(error);
            }
        }
        if let Err(error) = preference.save(self.profile().directory()) {
            if matches!(error, RuntimeError::Profile(ProfileError::CommitUncertain))
                && previous.save(self.profile().directory()).is_err()
            {
                self.stop_network();
                return Err(ProfileError::CommitUncertain.into());
            }
            self.restore_network(running.as_ref());
            return Err(error);
        }
        Ok(())
    }
    fn restore_network(&self, previous: Option<&RelaySetting>) {
        self.stop_network();
        if let Some(previous) = previous {
            // The original error remains authoritative; failed recovery leaves the
            // lifetime stopped so the client cannot mistake it for a working route.
            if self.start_network(previous.clone()).is_err() {
                self.stop_network();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn absent_settings_default_to_public_and_fixed_custom_choice_survives_reload() {
        let directory = tempfile::tempdir().unwrap();
        assert!(RelayPreference::load(directory.path()).unwrap() == RelayPreference::Public);
        let preference = RelayPreference::Custom("https://relay.example.test".into());
        preference.save(directory.path()).unwrap();
        assert!(RelayPreference::load(directory.path()).unwrap() == preference);
        assert_eq!(
            std::fs::read(directory.path().join(SETTINGS_FILE)).unwrap(),
            b"{\"mode\":\"custom\",\"url\":\"https://relay.example.test\"}"
        );
    }
    #[test]
    fn invalid_choices_and_damaged_settings_are_preserved() {
        let directory = tempfile::tempdir().unwrap();
        RelayPreference::Disabled.save(directory.path()).unwrap();
        let before = std::fs::read(directory.path().join(SETTINGS_FILE)).unwrap();
        assert!(
            RelayPreference::Custom("http://relay.example.test".into())
                .save(directory.path())
                .is_err()
        );
        assert_eq!(
            std::fs::read(directory.path().join(SETTINGS_FILE)).unwrap(),
            before
        );
        std::fs::write(
            directory.path().join(SETTINGS_FILE),
            b"PUBLIC invalid settings",
        )
        .unwrap();
        assert!(RelayPreference::load(directory.path()).is_err());
        assert_eq!(
            std::fs::read(directory.path().join(SETTINGS_FILE)).unwrap(),
            b"PUBLIC invalid settings"
        );
    }
    #[cfg(feature = "ui-test-support")]
    #[test]
    fn failed_relay_switch_restores_the_running_direct_lifetime_and_saved_choice() {
        let directory = tempfile::tempdir().unwrap();
        let host = RuntimeHost::with_test_sessions(
            directory.path(),
            crate::session::SessionController::new(Default::default()),
        )
        .unwrap();
        host.set_relay_preference(RelayPreference::Disabled)
            .unwrap();
        host.start_configured_network().unwrap();
        assert!(
            host.set_relay_preference(RelayPreference::Custom("https://127.0.0.1:9".into()))
                .is_err()
        );
        assert!(host.relay_preference().unwrap() == RelayPreference::Disabled);
        assert!(host.network_snapshot().unwrap().running);
        host.stop_configured_network();
        drop(host);
        let reopened = RuntimeHost::with_test_sessions(
            directory.path(),
            crate::session::SessionController::new(Default::default()),
        )
        .unwrap();
        assert!(reopened.relay_preference().unwrap() == RelayPreference::Disabled);
        assert!(!reopened.network_snapshot().unwrap().running);
    }
}
