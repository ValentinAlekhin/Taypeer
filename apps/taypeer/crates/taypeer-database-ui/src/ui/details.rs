//! Stateless entry-tab composition shared by retained product views and previews.

use super::style::*;
use crate::ui_state::EntryContent;
use gpui_kit::{component::*, prelude::FluentBuilder, *};

pub(super) fn choose_icon(button: button::Button) -> button::Button {
    button
        .accessibility_label(tr("ui.choose_icon"))
        .child(div().text_size(rems(13. / 16.)).child(tr("ui.choose_icon")))
        .h(rems(2.))
        .w(rems(10.))
        .max_w_full()
        .flex_shrink_0()
        .truncate()
        .text_size(rems(13. / 16.))
}

pub(super) fn icon_control(
    content: &EntryContent,
    commands: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> AnyElement {
    h_flex()
        .flex_wrap()
        .gap_2()
        .child(super::images::stored_icon(
            &content.icon,
            content.icon_blob.as_ref(),
            cx,
        ))
        .children(commands)
        .into_any_element()
}

pub(super) fn color_control(
    state: &Entity<color_picker::ColorPickerState>,
    background: bool,
    value: Option<u32>,
    reset: Option<AnyElement>,
) -> AnyElement {
    h_flex()
        .gap_2()
        .child(
            color_picker::ColorPicker::new(state)
                .label(color_text(value))
                .accessibility_label(tr(if background {
                    "ui.background_color"
                } else {
                    "ui.foreground_color"
                })),
        )
        .children(reset)
        .into_any_element()
}

pub(super) fn section_header(label: &str, actions: Option<AnyElement>) -> AnyElement {
    h_flex()
        .text_size(rems(1.))
        .line_height(rems(1.25))
        .justify_between()
        .pr_6()
        .child(section(label))
        .children(actions)
        .into_any_element()
}

pub(super) fn attribute_row(
    label: &str,
    value: impl IntoElement,
    actions: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    super::card::row_surface(
        false,
        row(
            label,
            h_flex()
                .min_w_0()
                .gap_2()
                .child(div().flex_1().min_w_0().truncate().child(value))
                .children(actions),
            cx,
        ),
        cx,
    )
}

pub(super) fn attachment_row(
    name: &str,
    details: Option<String>,
    actions: impl IntoElement,
    cx: &App,
) -> AnyElement {
    h_flex()
        .min_h(rems(2.75))
        .px_6()
        .gap_2()
        .text_size(rems(13. / 16.))
        .line_height(rems(1.25))
        .relative()
        .bg(cx.theme().background)
        .child(
            div()
                .absolute()
                .left_0()
                .right_0()
                .bottom_0()
                .h(px(1.))
                .bg(cx.theme().border),
        )
        .child(icon("file"))
        .child(div().flex_1().min_w_0().truncate().child(name.to_owned()))
        .when_some(details, |el, details| {
            el.child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(details),
            )
        })
        .child(actions)
        .into_any_element()
}

pub(super) fn attachments_hint(cx: &App) -> AnyElement {
    div()
        .p_6()
        .text_xs()
        .text_color(cx.theme().muted_foreground)
        .child(tr("ui.attachments_hint"))
        .into_any_element()
}

pub(super) fn appearance(
    content: &EntryContent,
    icon_control: impl IntoElement,
    foreground: impl IntoElement,
    background: impl IntoElement,
    cx: &App,
) -> AnyElement {
    v_flex()
        .bg(cx.theme().background)
        .child(row("ui.icon", icon_control, cx))
        .child(row("ui.foreground_color", foreground, cx))
        .child(row("ui.background_color", background, cx))
        .child(section_header("ui.preview", None))
        .child(
            div()
                .id("entry-appearance-preview")
                .test_support()
                .mx_6()
                .p_4()
                .rounded_sm()
                .bg(content
                    .background
                    .map(|c| rgba(c).into())
                    .unwrap_or(cx.theme().background))
                .text_color(
                    content
                        .foreground
                        .map(|c| rgba(c).into())
                        .unwrap_or(cx.theme().foreground),
                )
                .child(
                    h_flex()
                        .gap_2()
                        .child(super::images::stored_icon(
                            &content.icon,
                            content.icon_blob.as_ref(),
                            cx,
                        ))
                        .child(content.title.clone()),
                ),
        )
        .into_any_element()
}

pub(super) fn properties(values: [AnyElement; 3], cx: &App) -> AnyElement {
    v_flex()
        .children(
            ["ui.created", "ui.modified", "ui.id"]
                .into_iter()
                .zip(values)
                .enumerate()
                .map(|(index, (key, value))| {
                    super::card::row_surface(!index.is_multiple_of(2), row(key, value, cx), cx)
                }),
        )
        .into_any_element()
}
