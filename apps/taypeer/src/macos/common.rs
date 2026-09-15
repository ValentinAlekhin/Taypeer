//! Small stateless presentation helpers shared by screens.

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use rust_i18n::t;

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
