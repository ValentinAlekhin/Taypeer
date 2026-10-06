//! Row presentation over Kit's public ordinary-text editing engine.

use super::style::tr;
use gpui_kit::{
    base::{
        InputBase, Textarea as BaseTextarea,
        input::{Copy, Cut, InputEditorStyle, Paste, SelectAll},
    },
    component::{ActiveTheme, input::TextareaState, native_menu::NativeMenu},
    *,
};
use std::rc::Rc;

#[derive(IntoElement)]
pub(super) struct RowTextarea {
    state: Entity<TextareaState>,
    label: &'static str,
    readonly: bool,
    disabled: bool,
}

pub(super) fn control(
    state: &Entity<TextareaState>,
    label: &'static str,
    readonly: bool,
    disabled: bool,
) -> RowTextarea {
    RowTextarea {
        state: state.clone(),
        label,
        readonly,
        disabled,
    }
}

impl RenderOnce for RowTextarea {
    fn render(self, window: &mut Window, cx: &mut App) -> impl IntoElement {
        let theme = cx.theme();
        let colors = InputEditorStyle {
            foreground: theme.foreground,
            muted_foreground: theme.muted_foreground,
            background: theme.background,
            border: theme.border,
            selection: theme.selection,
            caret: theme.caret,
            ..Default::default()
        };
        self.state.update(cx, |state, cx| {
            state.set_editor_style(colors);
            state.set_editor_paddings(Edges::default());
            state.set_disabled(self.disabled, cx);
            state.set_readonly(self.readonly, cx);
            state.on_context_menu(Rc::new(|_, capabilities, position, window, cx| {
                let enabled = !capabilities.is_disabled();
                let editable = enabled && !capabilities.is_readonly();
                NativeMenu::new()
                    .menu_with_disabled(
                        tr("ui.cut"),
                        !(editable && capabilities.is_copyable()),
                        Box::new(Cut),
                    )
                    .menu_with_disabled(
                        tr("ui.copy"),
                        !(enabled && capabilities.is_copyable()),
                        Box::new(Copy),
                    )
                    .menu_with_disabled(
                        tr("ui.paste"),
                        !(editable && cx.read_from_clipboard().is_some()),
                        Box::new(Paste),
                    )
                    .separator()
                    .menu(tr("ui.select_all"), Box::new(SelectAll))
                    .show(position, window, cx);
            }));
        });
        let focus = self.state.focus_handle(cx);
        // The editing engine owns its own focus node. A separate frame scope
        // preserves its native action path, just as Kit's styled Input does.
        let frame_focus = window
            .use_keyed_state(
                ("row-textarea-frame", self.state.entity_id()),
                cx,
                |_, cx| cx.focus_handle(),
            )
            .read(cx)
            .clone();
        let value = self.state.read(cx).value();
        let target = self.state.clone();
        InputBase::new(("row-textarea", self.state.entity_id()))
            .focused(focus.is_focused(window))
            .disabled(self.disabled)
            .track_focus(&frame_focus)
            .role(Role::MultilineTextInput)
            .accessibility_label(tr(self.label))
            .aria_value(value)
            .w_full()
            .min_h(rems(2.))
            .py(rems(0.375))
            .text_size(rems(13. / 16.))
            .line_height(rems(1.25))
            .on_a11y_action(AccessibleAction::SetValue, move |data, window, cx| {
                if let Some(accesskit::ActionData::Value(value)) = data {
                    target.update(cx, |state, cx| {
                        if state.presentation().is_editable() {
                            state.replace_all(value.to_string(), window, cx);
                        }
                    });
                }
            })
            .child(BaseTextarea::new(&self.state))
    }
}
