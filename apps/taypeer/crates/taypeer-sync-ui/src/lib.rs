//! Exchange workflows and independently retained network presentation.
mod model;
mod relay;
pub use model::*;
pub use relay::configure_relay;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod host;
#[cfg(any(target_os = "macos", target_os = "linux"))]
mod view;
#[cfg(any(target_os = "macos", target_os = "linux"))]
pub use view::{SyncView, share};

#[cfg(any(target_os = "macos", target_os = "linux"))]
pub mod state;
