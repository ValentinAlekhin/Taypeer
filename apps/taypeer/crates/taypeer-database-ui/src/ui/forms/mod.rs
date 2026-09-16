//! Database-specific forms over the shared text-form primitive.
use super::style::*;
use super::workspace::WorkspaceStore;
use crate::ui_state::*;
use gpui_kit::*;
pub(super) use taypeer_ui::forms::{Deferred, Done, text_form, text_form_with_icon};

mod binary;
mod database;
mod entry;
mod group;
pub(super) use binary::{attachment, export_attachment, image_file, image_url};
pub use database::choose_file;
pub(super) use database::database;
pub(super) use entry::attribute;
pub(super) use group::{clone_group, group, trash_group};

/// Resolve database and shared-form dirty state before application exit.
pub fn request_quit(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut App) {
    taypeer_ui::forms::request_quit(
        std::rc::Rc::new(move |_, cx| {
            store.update(cx, |store, cx| store.request_quit(cx));
        }),
        window,
        cx,
    );
}
