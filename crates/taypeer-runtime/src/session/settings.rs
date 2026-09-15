use super::SessionPolicy;
use crate::{RuntimeError, profile::ProfileError};
use serde::{Deserialize, Serialize};
use std::{
    fs::{File, OpenOptions},
    io::{Read, Write},
    path::Path,
};

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Stored {
    version: u16,
    policy: SessionPolicy,
}
/// Nonsecret local preferences. These operations never acquire native credentials.
pub struct SessionSettings;
impl SessionSettings {
    /// Missing settings use the documented default; corrupt/unknown settings fail explicitly.
    pub fn load(profile: &Path) -> Result<SessionPolicy, RuntimeError> {
        let file = match File::open(profile.join("session-policy.json")) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(SessionPolicy::default());
            }
            Err(_) => return Err(ProfileError::Io.into()),
        };
        let mut bytes = Vec::new();
        file.take(4097)
            .read_to_end(&mut bytes)
            .map_err(|_| ProfileError::Io)?;
        if bytes.len() > 4096 {
            return Err(ProfileError::Invalid.into());
        }
        let stored: Stored = serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
        if stored.version != 1 {
            return Err(ProfileError::Invalid.into());
        }
        Ok(stored.policy)
    }
    /// Atomically save a validated policy under a separate local settings lock.
    pub fn save(profile: &Path, policy: SessionPolicy) -> Result<(), RuntimeError> {
        std::fs::create_dir_all(profile).map_err(|_| ProfileError::Io)?;
        let lock = OpenOptions::new()
            .create(true)
            .truncate(false)
            .read(true)
            .write(true)
            .open(profile.join("session-policy.lock"))
            .map_err(|_| ProfileError::Io)?;
        lock.try_lock().map_err(|_| ProfileError::Busy)?;
        let mut candidate =
            tempfile::NamedTempFile::new_in(profile).map_err(|_| ProfileError::Io)?;
        let bytes = serde_json::to_vec(&Stored { version: 1, policy })
            .map_err(|_| ProfileError::Invalid)?;
        candidate.write_all(&bytes).map_err(|_| ProfileError::Io)?;
        candidate
            .as_file()
            .sync_all()
            .map_err(|_| ProfileError::Io)?;
        candidate
            .persist(profile.join("session-policy.json"))
            .map_err(|_| ProfileError::Io)?;
        File::open(profile)
            .and_then(|file| file.sync_all())
            .map_err(|_| ProfileError::Io)?;
        Ok(())
    }
}
