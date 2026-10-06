//! Card chrome composed by the live inspector and retained component previews.

use super::style::tr;
use crate::ui_state::EntryField;
use gpui_kit::{
    component::{button::ButtonVariants, *},
    *,
};

pub(super) fn current_entry() -> button::Button {
    button::Button::new("current-entry")
        .ghost()
        .accessibility_label(tr("back"))
        .child(div().text_size(rems(13. / 16.)).child(tr("back")))
        .h(rems(2.))
        .text_size(rems(13. / 16.))
}

pub(super) fn frame(cx: &App) -> Div {
    v_flex().size_full().bg(cx.theme().background)
}

pub(super) fn body(content: impl IntoElement) -> Stateful<Div> {
    div()
        .id("inspector-scroll")
        .flex_1()
        .min_h_0()
        .overflow_y_scroll()
        .child(content)
}

pub(super) fn row_surface(alternate: bool, content: impl IntoElement, cx: &App) -> AnyElement {
    div()
        .w_full()
        .min_w_0()
        .bg(if alternate {
            cx.theme().button
        } else {
            cx.theme().background
        })
        .child(content)
        .into_any_element()
}

pub(super) fn overview_row(field: EntryField, content: impl IntoElement, cx: &App) -> AnyElement {
    row_surface(
        matches!(
            field,
            EntryField::Username | EntryField::Url | EntryField::Expires
        ),
        content,
        cx,
    )
}

pub(super) fn header(title: impl IntoElement, actions: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .id("entry-header")
        .test_support()
        .h(rems(2.75))
        .flex_shrink_0()
        .px_6()
        .gap_2()
        .relative()
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(1.))
                .bg(cx.theme().border),
        )
        .child(
            div()
                .id("entry-header-title")
                .test_support()
                .flex_1()
                .min_w_0()
                .truncate()
                .text_size(rems(18. / 16.))
                .line_height(rems(1.25))
                .font_weight(FontWeight::MEDIUM)
                .child(title),
        )
        .child(actions)
        .into_any_element()
}
