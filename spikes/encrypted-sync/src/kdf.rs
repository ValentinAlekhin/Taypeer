//! Measured KDF calibration, with bounded work. No artificial delay.
use crate::{MIN_MEMORY, Result, derive};
use serde::{Deserialize, Serialize};
use std::time::Instant;
#[derive(Clone, Copy, Serialize, Deserialize)]
pub struct Profile {
    pub memory: u32,
    pub iterations: u32,
}
impl Default for Profile {
    fn default() -> Self {
        Self {
            memory: MIN_MEMORY,
            iterations: 3,
        }
    }
}
impl Profile {
    pub fn measure(self) -> Result<u64> {
        let start = Instant::now();
        let _key = derive(
            b"PUBLIC calibration fixture",
            &[1; 16],
            self.memory,
            self.iterations,
        )?;
        Ok(start.elapsed().as_millis().max(1) as u64)
    }
}
#[derive(Serialize)]
pub struct Calibration {
    pub target_ms: u64,
    pub profile: Profile,
    pub measured_ms: u64,
    pub within_tolerance: bool,
}
pub fn calibrate(target_ms: u64) -> Result<Calibration> {
    if !(500..=5000).contains(&target_ms) {
        return Err("KDF target range");
    }
    let mut profile = Profile::default();
    let mut elapsed = profile.measure()?;
    // Limit calibration work to four measured KDF evaluations, each at bounded params.
    for _ in 0..3 {
        let iterations =
            ((profile.iterations as u64 * target_ms + elapsed / 2) / elapsed).clamp(3, 256) as u32;
        if iterations == profile.iterations {
            break;
        }
        profile.iterations = iterations;
        elapsed = profile.measure()?;
        if elapsed.abs_diff(target_ms) <= target_ms / 5 {
            break;
        }
    }
    Ok(Calibration {
        target_ms,
        profile,
        measured_ms: elapsed,
        within_tolerance: elapsed.abs_diff(target_ms) <= target_ms / 5,
    })
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn rejects_unbounded_target_before_kdf() {
        for n in [0, 499, 5001, u64::MAX] {
            assert!(calibrate(n).is_err());
        }
    }
}
