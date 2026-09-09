//! Synthetic UI/FFI lifetime probe. This is not a vault or an authentication system.
//! No persistence, networking, KDF or real keys are implemented here.

use std::sync::{Arc, Mutex, MutexGuard};
use zeroize::{Zeroize, Zeroizing};

uniffi::setup_scaffolding!();

/// Same embedded TOML for both frontends. Invalid/missing roles fail explicitly.
#[uniffi::export]
pub fn theme_palette(dark: bool) -> Result<std::collections::HashMap<String, u32>, ProbeError> {
    let palettes: std::collections::HashMap<String, std::collections::HashMap<String, u32>> =
        toml::from_str(include_str!("../palette.toml")).map_err(|_| ProbeError::Unavailable)?;
    palettes
        .get(if dark { "dark" } else { "light" })
        .cloned()
        .ok_or(ProbeError::Unavailable)
}

#[derive(Debug, thiserror::Error, uniffi::Error)]
pub enum ProbeError {
    #[error("session is locked")]
    Locked,
    #[error("request belongs to an expired session")]
    Expired,
    #[error("synthetic input must contain 1 to 256 characters")]
    InvalidInput,
    #[error("probe state is unavailable")]
    Unavailable,
}

#[derive(Clone, Debug, uniffi::Record)]
pub struct ProbeStatus {
    pub generation: u64,
    pub locked: bool,
    pub characters: u32,
}

#[derive(uniffi::Record)]
pub struct ProbeReply {
    pub generation: u64,
    pub text: String,
}

#[derive(Default)]
struct State {
    generation: u64,
    sample: Option<Zeroizing<Vec<u8>>>,
}

#[derive(Default, uniffi::Object)]
pub struct ProbeSession {
    state: Mutex<State>,
}

#[uniffi::export]
impl ProbeSession {
    #[uniffi::constructor]
    pub fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Starts a synthetic session with arbitrary sample text. Does NOT authenticate.
    pub fn open_sample(&self, mut text: String) -> Result<ProbeStatus, ProbeError> {
        let count = text.chars().count();
        if !(1..=256).contains(&count) {
            text.zeroize();
            return Err(ProbeError::InvalidInput);
        }
        let sample = Zeroizing::new(text.into_bytes());
        let mut state = self.state()?;
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(ProbeError::Unavailable)?;
        state.sample = Some(sample);
        Ok(status(&state))
    }

    /// Invalidates all old requests and drops/zeroizes the Rust-owned sample.
    pub fn lock(&self) -> Result<ProbeStatus, ProbeError> {
        let mut state = self.state()?;
        // Clear before handling even an exhausted generation counter.
        state.sample = None;
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(ProbeError::Unavailable)?;
        Ok(status(&state))
    }

    pub fn status(&self) -> Result<ProbeStatus, ProbeError> {
        let state = self.state()?;
        Ok(status(&state))
    }

    /// Checks the generation at completion, under the same lock as the data read.
    pub fn reveal(&self, generation: u64) -> Result<ProbeReply, ProbeError> {
        let state = self.state()?;
        let sample = state.sample.as_ref().ok_or(ProbeError::Locked)?;
        if generation != state.generation {
            return Err(ProbeError::Expired);
        }
        let text = String::from_utf8(sample.to_vec()).map_err(|_| ProbeError::Unavailable)?;
        Ok(ProbeReply { generation, text })
    }
}

impl ProbeSession {
    fn state(&self) -> Result<MutexGuard<'_, State>, ProbeError> {
        self.state.lock().map_err(|_| ProbeError::Unavailable)
    }
}

fn status(state: &State) -> ProbeStatus {
    ProbeStatus {
        generation: state.generation,
        locked: state.sample.is_none(),
        characters: state.sample.as_ref().map_or(0, |value| {
            std::str::from_utf8(value)
                .expect("sample was created from UTF-8")
                .chars()
                .count() as u32
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "SYNTHETIC-ONLY-Жук-42";

    #[test]
    fn both_palettes_have_all_roles_and_valid_rgb_colors() {
        let expected = [
            "background",
            "panel",
            "foreground",
            "muted",
            "border",
            "primary",
            "on_primary",
            "selection",
            "focus",
            "danger",
            "warning",
            "success",
            "disabled",
        ];
        for dark in [false, true] {
            let palette = theme_palette(dark).unwrap();
            assert_eq!(palette.len(), expected.len());
            for role in expected {
                assert!(palette[role] <= 0xffffff);
            }
        }
    }

    #[test]
    fn unicode_round_trip_is_exact_and_count_is_not_bytes() {
        let session = ProbeSession::new();
        let opened = session.open_sample(SAMPLE.into()).unwrap();
        assert_eq!(opened.characters, SAMPLE.chars().count() as u32);
        assert_eq!(session.reveal(opened.generation).unwrap().text, SAMPLE);
    }

    #[test]
    fn lock_removes_sample_and_denies_delayed_read() {
        let session = ProbeSession::new();
        let opened = session.open_sample(SAMPLE.into()).unwrap();
        let locked = session.lock().unwrap();
        assert!(locked.locked);
        assert_eq!(locked.characters, 0);
        assert!(session.state().unwrap().sample.is_none());
        assert!(matches!(
            session.reveal(opened.generation),
            Err(ProbeError::Locked)
        ));
    }

    #[test]
    fn old_request_cannot_read_new_session() {
        let session = ProbeSession::new();
        let old = session.open_sample(SAMPLE.into()).unwrap();
        session.lock().unwrap();
        let new = session.open_sample("SYNTHETIC-SECOND".into()).unwrap();
        assert!(matches!(
            session.reveal(old.generation),
            Err(ProbeError::Expired)
        ));
        assert_eq!(
            session.reveal(new.generation).unwrap().text,
            "SYNTHETIC-SECOND"
        );
    }

    #[test]
    fn replacing_session_also_invalidates_old_request() {
        let session = ProbeSession::new();
        let old = session.open_sample(SAMPLE.into()).unwrap();
        session.open_sample("SYNTHETIC-REPLACEMENT".into()).unwrap();
        assert!(matches!(
            session.reveal(old.generation),
            Err(ProbeError::Expired)
        ));
    }

    #[test]
    fn invalid_input_does_not_change_existing_session() {
        let session = ProbeSession::new();
        let opened = session.open_sample(SAMPLE.into()).unwrap();
        for invalid in [String::new(), "Ж".repeat(257)] {
            assert!(matches!(
                session.open_sample(invalid),
                Err(ProbeError::InvalidInput)
            ));
        }
        assert_eq!(session.reveal(opened.generation).unwrap().text, SAMPLE);
    }

    #[test]
    fn repeated_lock_stays_locked_and_invalidates_issued_reply() {
        let session = ProbeSession::new();
        let opened = session.open_sample(SAMPLE.into()).unwrap();
        let reply = session.reveal(opened.generation).unwrap();
        session.lock().unwrap();
        let locked = session.lock().unwrap();
        // UI must check this too: an already returned copy cannot be revoked by Rust.
        assert_ne!(reply.generation, locked.generation);
        assert!(locked.locked);
    }
}
