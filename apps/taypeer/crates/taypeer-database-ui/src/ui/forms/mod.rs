//! Database-specific forms over the shared text-form primitive.
use super::style::*;
use super::workspace::WorkspaceStore;
use crate::ui_state::*;
use gpui_kit::*;
pub(super) use taypeer_ui::forms::{Deferred, Done, text_form};

mod binary;
mod database;
mod entry;
mod group;
pub(in crate::ui) mod metadata;
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

/// Owns the identity of one exact metadata-form submission.
#[derive(Default)]
struct FormAttempt(std::cell::RefCell<Option<(Vec<String>, taypeer_core::OperationId)>>);
impl FormAttempt {
    fn operation(&self, values: &[String]) -> Result<taypeer_core::OperationId, FormError> {
        let mut attempt = self.0.borrow_mut();
        if let Some((old, operation)) = &*attempt
            && old == values
        {
            return Ok(operation.clone());
        }
        let operation = taypeer_services::new_operation_id().map_err(|_| FormError::Backend)?;
        *attempt = Some((values.to_vec(), operation.clone()));
        Ok(operation)
    }
}
