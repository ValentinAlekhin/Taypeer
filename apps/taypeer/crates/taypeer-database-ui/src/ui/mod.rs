//! Database views, commands and workflow owners.
mod database_settings;
mod editor;
mod entries;
mod forms;
mod generator;
mod header;
mod images;
mod inspector;
mod read_value;
mod session;
mod sidebar;
mod workspace;
pub use database_settings::DatabaseSettings;
pub use entries::Entries;
pub use forms::{choose_file, request_quit};
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
