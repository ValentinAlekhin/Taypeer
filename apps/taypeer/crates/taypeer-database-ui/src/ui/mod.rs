//! Database views, commands and workflow owners.
mod card;
#[cfg(feature = "component-rendering")]
pub mod component_previews;
mod database_settings;
mod details;
mod editor;
mod entries;
mod forms;
mod generator;
mod header;
mod history;
mod images;
mod inspector;
mod metadata_history;
mod read_value;
mod row_textarea;
mod session;
mod sidebar;
mod workspace;
pub use database_settings::DatabaseSettings;
pub use entries::Entries;
#[cfg(feature = "component-rendering")]
pub use forms::metadata_preview;
pub use forms::{choose_file, request_quit};
#[cfg(feature = "component-rendering")]
pub use generator::{GeneratorPreviewCase, generator_preview};
pub use header::Header;
pub use inspector::Inspector;
pub use session::SessionView;
pub use sidebar::Sidebar;
use taypeer_sync_ui as sync;
use taypeer_ui::{clipboard, icons, style};
pub use workspace::WorkspaceStore;
/// Initialize the database-owned image cache once per application.
pub fn init(cx: &mut gpui_kit::App) {
    cx.set_global(images::Images::default());
}
