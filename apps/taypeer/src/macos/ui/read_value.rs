//! Selectable, read-only values scoped to the displayed entry.
use super::{clipboard, style::tr, workspace::WorkspaceStore};
use crate::ui_state::{DatabaseId, EntryId};
use gpui_kit::{
    component::input::{Copy, Cut, Textarea, TextareaState},
    *,
};

pub(super) struct ReadValue {
    input: Entity<TextareaState>,
}

impl ReadValue {
    pub fn new(value: &str, window: &mut Window, cx: &mut App) -> Self {
        let input = cx.new(|cx| {
            let mut input = TextareaState::new(window, cx).auto_grow(1, 20);
            input.set_value(value, window, cx);
            input
        });
        Self { input }
    }

    pub fn sync(&self, value: &str, window: &mut Window, cx: &mut App) {
        if self.input.read(cx).value().as_str() != value {
            self.input
                .update(cx, |input, cx| input.set_value(value, window, cx));
        }
    }

    pub fn clear(&self, window: &mut Window, cx: &mut App) {
        self.input
            .update(cx, |input, cx| input.set_value("", window, cx));
    }

    pub fn render(
        &self,
        sensitive: bool,
        store: Entity<WorkspaceStore>,
        identity: Option<(DatabaseId, EntryId)>,
    ) -> AnyElement {
        let input = self.input.clone();
        div()
            .w_full()
            .child(
                Textarea::new(&self.input)
                    .readonly(true)
                    .appearance(false)
                    .bordered(false)
                    .aria_label(tr("value")),
            )
            .capture_action(move |_: &Copy, _, cx| {
                let current = store.read(cx);
                let valid = identity.as_ref().is_some_and(|(db, entry)| {
                    current.state().database.as_ref() == Some(db)
                        && current.state().selected.as_ref() == Some(entry)
                        && current.connection().is_some_and(|c| c.control.is_open())
                });
                if valid {
                    let text = input.read(cx).selected_value().to_string();
                    if !text.is_empty() {
                        clipboard::copy_selection(text, sensitive, cx);
                    }
                }
                cx.stop_propagation();
            })
            .capture_action(|_: &Cut, _, cx| cx.stop_propagation())
            .into_any_element()
    }
}
