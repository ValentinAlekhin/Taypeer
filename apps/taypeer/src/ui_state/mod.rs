//! Portable state for the synthetic UI. No database service, filesystem or GPUI.

mod catalog;
mod draft;
mod fixtures;
mod navigation;

pub(crate) use catalog::*;
pub(crate) use draft::*;
pub(crate) use navigation::*;

#[cfg(test)]
mod tests;
