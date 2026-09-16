//! Exchange workflows and independently retained network presentation.
mod model;
mod relay;
pub use model::*;
pub use relay::configure_relay;
#[cfg(target_os = "macos")]
pub mod host;
#[cfg(target_os = "macos")]
mod view;
#[cfg(target_os = "macos")]
pub use view::{SyncView, share};

#[cfg(target_os = "macos")]
pub mod state;
