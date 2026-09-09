//! Small stateless presentation helpers shared by screens.

use gpui_kit::component::input::InputState;
use gpui_kit::component::*;
use gpui_kit::*;
use rust_i18n::t;

pub(super) fn format_date(value: i64) -> String {
    chrono::DateTime::from_timestamp_millis(value)
        .map(|v| v.format("%Y-%m-%d %H:%M").to_string())
        .unwrap_or_default()
}

pub(super) fn tr(key: &str) -> SharedString {
    t!(key).to_string().into()
}
pub(super) fn input(
    value: &str,
    masked: bool,
    window: &mut Window,
    cx: &mut App,
) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).masked(masked);
        state.set_value(value, window, cx);
        state
    })
}

pub(super) fn field_row(label: &str, value: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .min_h_11()
        .gap_4()
        .border_b_1()
        .border_color(cx.theme().border)
        .items_start()
        .py_2()
        .child(
            div()
                .w(rems(9.))
                .flex_shrink_0()
                .text_color(cx.theme().muted_foreground)
                .child(tr(label)),
        )
        .child(div().flex_1().min_w_0().child(value))
        .into_any_element()
}
