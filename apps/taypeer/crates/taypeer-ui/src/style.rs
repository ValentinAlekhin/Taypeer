//! Product presentation primitives shared by the new screens.

pub use crate::common::{input, tr};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState},
        *,
    },
    *,
};

/// Resolve an embedded product icon at the relative icon size.
pub fn icon(name: &str) -> Icon {
    Icon::default()
        .path(SharedString::from(format!("ui-icons/{name}.svg")))
        .size(rems(1.))
}
/// Build a quiet command button with localized tooltip and accessibility label.
pub fn icon_button(id: impl Into<ElementId>, name: &str, label: &str) -> Button {
    Button::new(id)
        .ghost()
        .compact()
        .icon(icon(name))
        .tooltip(tr(label))
        .accessibility_label(tr(label))
        .h(rems(2.))
        .w(rems(2.))
}
/// Compose a labeled read-only row with a shrinkable value region.
pub fn row(label: &str, value: impl IntoElement, cx: &App) -> AnyElement {
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
/// Render an owned input with a stable resource-key ID and accessible name.
pub fn field(state: &Entity<InputState>, label: &str) -> Input {
    Input::new(state)
        .id(SharedString::from(format!("field-{label}")))
        .aria_label(tr(label))
        .bordered(false)
        .focus_bordered(true)
        .h(rems(2.))
}
/// Shared underline tabs for entry details and settings.
pub fn tabs(id: &'static str, labels: &[&'static str], selected: usize, cx: &App) -> tab::TabBar {
    tab::TabBar::new(id)
        .underline()
        .selected_index(selected)
        .h(rems(2.375))
        .border_b_1()
        .border_color(cx.theme().border)
        .children(
            labels
                .iter()
                .map(|key| tab::Tab::new().px(rems(0.75)).label(tr(key))),
        )
}

/// A form label focuses its associated control without changing its value.
pub fn input_row(label: &str, focus: FocusHandle, value: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .min_h(rems(2.75))
        .px(rems(1.5))
        .gap(rems(1.))
        .border_b_1()
        .border_color(cx.theme().border)
        .child(
            div()
                .id(SharedString::from(format!("label-{label}")))
                .w(rems(9.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(tr(label))
                .on_click(move |_, window, cx| focus.focus(window, cx)),
        )
        .child(div().flex_1().min_w_0().py_1().child(value))
        .into_any_element()
}

/// Format a database-supplied RGBA value without interpreting it as a theme role.
pub fn color_text(color: Option<u32>) -> String {
    color.map_or_else(
        || "—".into(),
        |color| {
            if color & 255 == 255 {
                format!("#{:06X}", color >> 8)
            } else {
                format!("#{color:08X}")
            }
        },
    )
}

/// Render a localized empty state inside its owning viewport.
pub fn empty(title: &str, cx: &App) -> AnyElement {
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
/// Render a localized section heading.
pub fn section(title: &str) -> AnyElement {
    div()
        .px(rems(1.5))
        .py_4()
        .font_weight(FontWeight::SEMIBOLD)
        .child(tr(title))
        .into_any_element()
}
/// Format a service timestamp in UTC, retaining millisecond precision.
pub fn stamp(time: i64) -> String {
    chrono::DateTime::from_timestamp_millis(time)
        .map(|v| v.format("%Y-%m-%d %H:%M:%S%.3f").to_string())
        .unwrap_or_default()
}

/// Decimal file sizes, matching macOS: 1 KB = 1,000 bytes.
pub fn file_size(bytes: u64) -> String {
    let units = [
        "ui.size_bytes",
        "ui.size_kb",
        "ui.size_mb",
        "ui.size_gb",
        "ui.size_tb",
        "ui.size_pb",
        "ui.size_eb",
    ];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1000. && unit < units.len() - 1 {
        value /= 1000.;
        unit += 1;
    }
    // Promote a rounded boundary instead of displaying "1000 KB".
    value = (value * 10.).round() / 10.;
    if value >= 1000. && unit < units.len() - 1 {
        value /= 1000.;
        unit += 1;
    }
    let number = format!("{value:.1}");
    let number = number
        .strip_suffix(".0")
        .unwrap_or(&number)
        .replace('.', tr("ui.decimal_separator").as_ref());
    format!("{number}\u{a0}{}", tr(units[unit]))
}

/// Kit's plain Dialog requires an explicit footer; actions retain its focus trap.
pub fn dialog_actions() -> AnyElement {
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
