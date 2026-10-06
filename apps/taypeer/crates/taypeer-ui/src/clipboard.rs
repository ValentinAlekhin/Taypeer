//! One clipboard boundary for ordinary text and explicitly requested secrets.
use gpui_kit::*;
/// Request a native clipboard write with visible completion feedback.
pub fn copy(value: String, sensitive: bool, cx: &mut App) {
    if let Some(platform) = cx.try_global::<taypeer_desktop_platform::Platform>() {
        platform.copy(value, sensitive, true);
    }
}

/// Copy selected text without a success toast; native failures remain visible.
pub fn copy_selection(value: String, sensitive: bool, cx: &mut App) {
    if let Some(platform) = cx.try_global::<taypeer_desktop_platform::Platform>() {
        platform.copy(value, sensitive, false);
    }
}

/// Route keyboard and menu copy/cut from secret inputs through the same sensitive pasteboard writer.
pub fn secret_field(
    input: impl gpui_kit::IntoElement,
    state: &gpui_kit::Entity<gpui_kit::component::input::InputState>,
    secret: bool,
) -> gpui_kit::AnyElement {
    protect_input(input, SecretControl::Input(state.clone()), secret)
}

/// Apply the same sensitive copy/cut boundary to a retained multiline input.
pub fn secret_textarea(
    input: impl gpui_kit::IntoElement,
    state: &gpui_kit::Entity<gpui_kit::component::input::TextareaState>,
    secret: bool,
) -> gpui_kit::AnyElement {
    protect_input(input, SecretControl::Textarea(state.clone()), secret)
}

#[derive(Clone)]
enum SecretControl {
    Input(Entity<gpui_kit::component::input::InputState>),
    Textarea(Entity<gpui_kit::component::input::TextareaState>),
}
impl SecretControl {
    fn selected(&self, cx: &App) -> String {
        match self {
            Self::Input(input) => input.read(cx).selected_value().to_string(),
            Self::Textarea(input) => input.read(cx).selected_value().to_string(),
        }
    }
    fn editable(&self, cx: &App) -> bool {
        match self {
            Self::Input(input) => input.read(cx).is_editable(),
            Self::Textarea(input) => input.read(cx).is_editable(),
        }
    }
    fn erase(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Input(input) => input.update(cx, |input, cx| input.replace("", window, cx)),
            Self::Textarea(input) => input.update(cx, |input, cx| input.replace("", window, cx)),
        }
    }
}

fn protect_input(input: impl IntoElement, state: SecretControl, secret: bool) -> AnyElement {
    use gpui_kit::{
        component::input::{Copy, Cut},
        prelude::FluentBuilder,
        *,
    };
    let copy = state.clone();
    let cut = state;
    div()
        .w_full()
        .child(input)
        .when(secret, |el| {
            el.capture_action(move |_: &Copy, _, cx| {
                let text = copy.selected(cx);
                if !text.is_empty() {
                    self::copy_selection(text, true, cx);
                }
                cx.stop_propagation();
            })
            .capture_action(move |_: &Cut, window, cx| {
                if cut.editable(cx) {
                    let text = cut.selected(cx);
                    if !text.is_empty() {
                        self::copy(text, true, cx);
                        cut.erase(window, cx);
                    }
                }
                cx.stop_propagation();
            })
        })
        .into_any_element()
}
