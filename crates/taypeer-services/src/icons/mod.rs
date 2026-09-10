//! Explicit bounded image acquisition; ordinary database reads never call this module.

mod http;
mod validation;

use serde::{Deserialize, Serialize};
use std::{fs::File, io::Read, path::Path};
use taypeer_core::ICON_LIMIT;
use zeroize::Zeroizing;

/// Image acquisition failures without URLs, response bodies or decoder diagnostics.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum IconError {
    /// Only HTTP(S) URLs without embedded credentials are accepted.
    InvalidUrl,
    /// A public site attempted to select an internal network destination.
    NetworkBoundary,
    /// Network request failed, including certificate verification.
    Network,
    /// The complete download exceeded its deadline.
    Timeout,
    /// Download, dimensions or decoding resources exceeded their limit.
    TooLarge,
    /// Image encoding is invalid or unsupported.
    InvalidImage,
    /// Selected file could not be read.
    File,
}

/// Validate original image bytes without loading external SVG resources.
pub fn validate(bytes: &[u8]) -> Result<(), IconError> {
    validation::validate(bytes)
}

/// Read and validate an explicitly selected icon; no path is stored as provenance.
pub fn from_file(path: &Path) -> Result<Zeroizing<Vec<u8>>, IconError> {
    let mut bytes = Zeroizing::new(Vec::new());
    File::open(path)
        .map_err(|_| IconError::File)?
        .take(ICON_LIMIT + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| IconError::File)?;
    validate(&bytes)?;
    Ok(bytes)
}

/// Download an explicitly requested original image with a 30-second aggregate deadline.
pub fn from_url(url: &str) -> Result<Zeroizing<Vec<u8>>, IconError> {
    http::download(url, false)
}

/// Find a site's declared icon or `/favicon.ico`, without a third-party icon service.
pub fn favicon(url: &str) -> Result<Zeroizing<Vec<u8>>, IconError> {
    http::download(url, true)
}

#[cfg(test)]
mod tests;
