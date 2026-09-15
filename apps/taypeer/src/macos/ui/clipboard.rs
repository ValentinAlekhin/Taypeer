//! One clipboard boundary for ordinary text and explicitly requested secrets.
use gpui_kit::*;
pub(super) fn copy(value: String, sensitive: bool, cx: &mut App) {
    if let Some(platform) = cx.try_global::<crate::macos::platform::Platform>() {
        platform.copy(value, sensitive);
    }
}

/// Route keyboard and menu copy/cut from secret inputs through the same sensitive pasteboard writer.
pub(super) fn secret_field(
    input: gpui_kit::component::input::Input,
    state: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
    secret: bool,
) -> gpui_kit::AnyElement {
    use gpui_kit::{
        component::input::{Copy, Cut},
        prelude::FluentBuilder,
        *,
    };
    let copy = state.clone();
    let cut = state.clone();
    div()
        .w_full()
        .child(input)
        .when(secret, |el| {
            el.capture_action(move |_: &Copy, _, cx| {
                let text = copy.read(cx).selected_value().to_string();
                if !text.is_empty() {
                    self::copy(text, true, cx);
                }
                cx.stop_propagation();
            })
            .capture_action(move |_: &Cut, window, cx| {
                if cut.read(cx).is_editable() {
                    let text = cut.read(cx).selected_value().to_string();
                    if !text.is_empty() {
                        self::copy(text, true, cx);
                        cut.update(cx, |input, cx| input.replace("", window, cx));
                    }
                }
                cx.stop_propagation();
            })
        })
        .into_any_element()
}
