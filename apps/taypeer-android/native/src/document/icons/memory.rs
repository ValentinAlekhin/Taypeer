//! Bounded ephemeral image capabilities. No plaintext temporary file or persistent cache.
use super::{AndroidError, IconFailure};
use crate::{SelectedInput, SelectedOutput};
use std::sync::Mutex;
use taypeer_core::ICON_LIMIT;
use zeroize::{Zeroize, Zeroizing};

pub(in crate::document) struct ImageInput(Mutex<Zeroizing<Vec<u8>>>);
impl ImageInput {
    pub(in crate::document) fn new(bytes: Zeroizing<Vec<u8>>) -> Self {
        Self(Mutex::new(bytes))
    }
}
impl SelectedInput for ImageInput {
    fn length(&self) -> Result<u64, AndroidError> {
        Ok(self.0.lock().map_err(super::super::failure)?.len() as u64)
    }
    fn read(&self, offset: u64, count: u32) -> Result<Vec<u8>, AndroidError> {
        if count as usize > crate::descriptors::TRANSFER_CHUNK {
            return Err(AndroidError::InvalidOptions);
        }
        let bytes = self.0.lock().map_err(super::super::failure)?;
        let offset = usize::try_from(offset).map_err(|_| AndroidError::InvalidOptions)?;
        let rest = bytes.get(offset..).ok_or(AndroidError::InvalidOptions)?;
        Ok(rest[..rest.len().min(count as usize)].to_vec())
    }
    fn close(&self) {
        if let Ok(mut bytes) = self.0.lock() {
            bytes.zeroize();
        }
    }
}

#[derive(Default)]
struct OutputState {
    bytes: Zeroizing<Vec<u8>>,
    finished: bool,
    closed: bool,
}
#[derive(Default)]
pub(super) struct ImageOutput(Mutex<OutputState>);
impl ImageOutput {
    pub(super) fn take(&self) -> Result<Zeroizing<Vec<u8>>, AndroidError> {
        let mut state = self.0.lock().map_err(super::super::failure)?;
        if !state.finished || state.closed {
            return Err(AndroidError::InvalidFile);
        }
        Ok(Zeroizing::new(std::mem::take(&mut *state.bytes)))
    }
}
impl SelectedOutput for ImageOutput {
    fn write(&self, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError> {
        let bytes = Zeroizing::new(bytes);
        let mut state = self.0.lock().map_err(super::super::failure)?;
        if state.finished || state.closed || offset != state.bytes.len() as u64 {
            return Err(AndroidError::InvalidOptions);
        }
        let end = offset
            .checked_add(bytes.len() as u64)
            .ok_or(AndroidError::InvalidOptions)?;
        if end > ICON_LIMIT {
            return Err(AndroidError::Icon {
                failure: IconFailure::TooLarge,
            });
        }
        state.bytes.extend_from_slice(&bytes);
        Ok(bytes.len() as u32)
    }
    fn finish(&self) -> Result<(), AndroidError> {
        let mut state = self.0.lock().map_err(super::super::failure)?;
        if state.closed {
            return Err(AndroidError::InvalidOptions);
        }
        // For this ephemeral destination, finish publishes a complete RAM value,
        // not a database save or an external durable file export.
        state.finished = true;
        Ok(())
    }
    fn close(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.bytes.zeroize();
            state.closed = true;
        }
    }
}
