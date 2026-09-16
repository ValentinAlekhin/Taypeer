//! Small stateless presentation helpers shared by screens.

use gpui_kit::component::input::InputState;
use gpui_kit::*;
use rust_i18n::t;

/// Resolve a stable resource key in the active application locale.
pub fn tr(key: &str) -> SharedString {
    t!(key).to_string().into()
}
/// Create a retained input; its caller owns the entity and its lifecycle.
pub fn input(value: &str, masked: bool, window: &mut Window, cx: &mut App) -> Entity<InputState> {
    cx.new(|cx| {
        let mut state = InputState::new(window, cx).masked(masked);
        state.set_value(value, window, cx);
        state
    })
}
