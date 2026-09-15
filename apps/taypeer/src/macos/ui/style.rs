//! Product presentation primitives shared by the new screens.

pub(super) use crate::macos::common::{input, tr};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState},
        *,
    },
    *,
};

pub(super) fn icon(name: &str) -> Icon {
    Icon::default()
        .path(SharedString::from(format!("ui-icons/{name}.svg")))
        .size(rems(1.))
}
pub(super) fn icon_button(id: impl Into<ElementId>, name: &str, label: &str) -> Button {
    Button::new(id)
        .ghost()
        .compact()
        .icon(icon(name))
        .tooltip(tr(label))
        .accessibility_label(tr(label))
        .h(rems(2.))
        .w(rems(2.))
}
pub(super) fn row(label: &str, value: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .min_h(rems(2.75))
        .px(rems(1.5))
        .gap(rems(1.))
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .w(rems(9.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(tr(label)),
        )
        .child(div().flex_1().min_w_0().py_1().child(value))
        .into_any_element()
}
pub(super) fn field(state: &Entity<InputState>, label: &str) -> Input {
    Input::new(state)
        .aria_label(tr(label))
        .bordered(false)
        .focus_bordered(true)
        .h(rems(2.))
}
pub(super) fn empty(title: &str, cx: &App) -> AnyElement {
    v_flex()
        .flex_1()
        .items_center()
        .justify_center()
        .gap_3()
        .p_6()
        .text_color(cx.theme().muted_foreground)
        .child(tr(title))
        .into_any_element()
}
pub(super) fn section(title: &str) -> AnyElement {
    div()
        .px(rems(1.5))
        .py_4()
        .font_weight(FontWeight::SEMIBOLD)
        .child(tr(title))
        .into_any_element()
}
pub(super) fn stamp(sequence: u64) -> String {
    format!("2026-09-15 10:{:02}", sequence % 60)
}

/// Kit's plain Dialog requires an explicit footer; actions retain its focus trap.
pub(super) fn dialog_actions() -> AnyElement {
    h_flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new("dialog-cancel")
                .label(tr("cancel"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(dialog::Cancel), cx)),
        )
        .child(
            Button::new("dialog-confirm")
                .primary()
                .label(tr("ui.confirm"))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(dialog::Confirm { secondary: false }), cx)
                }),
        )
        .into_any_element()
}
