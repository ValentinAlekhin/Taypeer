//! Shared presentation of confirmed revisions for entries and metadata.

use super::style::*;
use gpui_kit::{
    component::{button::*, *},
    *,
};

pub(super) fn header(actions: impl IntoElement) -> AnyElement {
    h_flex()
        .px_6()
        .py_3()
        .text_size(rems(14. / 16.))
        .line_height(rems(1.25))
        .child(div().flex_1().child(tr("history")))
        .child(actions)
        .into_any_element()
}

pub(super) fn column_header(cx: &App) -> AnyElement {
    h_flex()
        .id("history-columns")
        .test_support()
        .h(rems(2.375))
        .px_6()
        .gap_4()
        .text_size(rems(12. / 16.))
        .line_height(rems(1.25))
        .text_color(cx.theme().muted_foreground)
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
                .w(rems(13.))
                .flex_shrink_0()
                .child(tr("ui.history_saved")),
        )
        .child(div().flex_1().min_w_0().child(tr("ui.history_title")))
        .into_any_element()
}

pub(super) fn version(
    id: impl Into<ElementId>,
    saved: i64,
    title: impl IntoElement,
    selected: bool,
    alternate: bool,
    cx: &App,
) -> Button {
    Button::new(id)
        .ghost()
        .selected(selected)
        .bg(if selected {
            cx.theme().list_active
        } else if alternate {
            cx.theme().button
        } else {
            cx.theme().background
        })
        .w_full()
        .min_h(rems(2.75))
        .p_0()
        .rounded_none()
        .child(
            h_flex()
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
                .w_full()
                .min_h(rems(2.75))
                .px_6()
                .gap_4()
                .text_size(rems(13. / 16.))
                .line_height(rems(1.25))
                .font_weight(FontWeight::NORMAL)
                .child(div().w(rems(13.)).flex_shrink_0().child(stamp(saved)))
                .child(
                    div()
                        .flex_1()
                        .min_w_0()
                        .text_color(cx.theme().muted_foreground)
                        .child(title),
                ),
        )
}

pub(super) fn command(id: impl Into<ElementId>, label: &str) -> Button {
    default_button(id)
        .accessibility_label(tr(label))
        .child(div().text_size(rems(13. / 16.)).child(tr(label)))
}

pub(super) fn commands(view: Button, compare: Button, restore: Button) -> AnyElement {
    h_flex()
        .flex_wrap()
        .px_6()
        .py_4()
        .gap_2()
        .children([view, compare, restore].map(|button| {
            button
                .h(rems(2.))
                .flex_shrink_0()
                .text_size(rems(13. / 16.))
        }))
        .into_any_element()
}
