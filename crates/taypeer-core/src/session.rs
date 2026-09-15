//! Device-local inactivity policy; independent of database semantics and platform clocks.
use serde::{Deserialize, Serialize};
use std::{num::NonZeroU32, time::Duration};

/// A positive inactivity interval shared by the device's open database sessions.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SessionPolicy {
    idle_seconds: NonZeroU32,
}
impl Default for SessionPolicy {
    fn default() -> Self {
        Self {
            idle_seconds: NonZeroU32::new(300).expect("default interval is positive"),
        }
    }
}
impl SessionPolicy {
    /// Zero is invalid; this policy does not offer an implicit disable switch.
    pub fn new(idle_seconds: u32) -> Option<Self> {
        Some(Self {
            idle_seconds: NonZeroU32::new(idle_seconds)?,
        })
    }
    /// The persisted interval in seconds.
    pub fn idle_seconds(self) -> u32 {
        self.idle_seconds.get()
    }
    /// Duration used by a platform's monotonic clock.
    pub fn idle_duration(self) -> Duration {
        Duration::from_secs(u64::from(self.idle_seconds()))
    }
}
