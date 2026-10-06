//! Selectable, read-only values scoped to the displayed entry.
use super::{clipboard, workspace::WorkspaceStore};
use crate::ui_state::{DatabaseId, EntryId};
use gpui_kit::{
    component::{
        input::{Copy, Cut, TextareaState},
        *,
    },
    prelude::FluentBuilder,
    *,
};

pub(super) struct ReadValue {
    input: Entity<TextareaState>,
}
pub(super) fn frame(
    id: impl Into<ElementId>,
    value: impl IntoElement,
    secret: bool,
    actions: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    h_flex()
        .id(id)
        .test_support()
        .group("entry-value")
        .gap_2()
        .min_w_0()
        .child(
            div()
                .flex_1()
                .min_w_0()
                .when(secret, |el| {
                    el.font_family(cx.theme().mono_font_family.clone())
                })
                .child(value),
        )
        .when_some(actions, |el, actions| {
            el.child(
                h_flex()
                    .gap_2()
                    .invisible()
                    .group_hover("entry-value", |style| style.visible())
                    .child(actions),
            )
        })
        .into_any_element()
}
pub(super) fn control(
    input: &Entity<TextareaState>,
    id: SharedString,
    value: String,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id(id)
        .test_support()
        .aria_label(value)
        .w_full()
        .child(super::row_textarea::control(input, "value", true, false))
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
        id: SharedString,
        value: String,
        sensitive: bool,
        store: Entity<WorkspaceStore>,
        identity: Option<(DatabaseId, EntryId)>,
    ) -> AnyElement {
        let input = self.input.clone();
        control(&self.input, id, value)
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
