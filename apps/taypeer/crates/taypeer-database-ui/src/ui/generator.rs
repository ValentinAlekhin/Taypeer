//! Explicit offline generation through the shared cryptographic generator.

use super::style::*;
use crate::ui_state::EditorStore;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        checkbox::Checkbox,
        input::{InputEvent, InputState},
        *,
    },
    *,
};

struct GeneratorView {
    editor: Entity<EditorStore>,
    length: Entity<InputState>,
    exclusions: Entity<InputState>,
    phrase: bool,
    sets: [bool; 4],
    revealed: bool,
    generated: Option<taypeer_services::generator::GeneratedSecret>,
    _subscriptions: Vec<Subscription>,
}
impl GeneratorView {
    fn new(editor: Entity<EditorStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let length = input("30", false, window, cx);
        let exclusions = input("", false, window, cx);
        let subscriptions = vec![
            cx.subscribe(&length, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.generate(cx);
                }
            }),
            cx.subscribe(&exclusions, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.generate(cx);
                }
            }),
        ];
        let mut result = Self {
            editor,
            length,
            exclusions,
            phrase: false,
            sets: [true; 4],
            revealed: true,
            generated: None,
            _subscriptions: subscriptions,
        };
        result.generate(cx);
        result
    }
    fn generate(&mut self, cx: &mut Context<Self>) {
        use taypeer_services::generator::{self, PasswordOptions};
        self.generated = if self.phrase {
            self.length
                .read(cx)
                .value()
                .parse()
                .ok()
                .and_then(|count| generator::passphrase(count, "-").ok())
        } else {
            self.length
                .read(cx)
                .value()
                .parse()
                .ok()
                .and_then(|length| {
                    generator::password(&PasswordOptions {
                        length,
                        uppercase: self.sets[0],
                        lowercase: self.sets[1],
                        digits: self.sets[2],
                        punctuation: self.sets[3],
                        exclude: self.exclusions.read(cx).value().to_string(),
                        ..Default::default()
                    })
                    .ok()
                })
        };
        cx.notify();
    }
}
impl Render for GeneratorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let valid = self.generated.is_some();
        v_flex()
            .capture_key_down(|_, _, cx| taypeer_desktop_platform::activity(cx))
            .capture_any_mouse_down(|_, _, cx| taypeer_desktop_platform::activity(cx))
            .gap_4()
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        div()
                            .flex_1()
                            .font_family(cx.theme().mono_font_family.clone())
                            .child(if self.revealed {
                                self.generated
                                    .as_ref()
                                    .map(|s| s.expose().to_owned())
                                    .unwrap_or_else(|| tr("ui.generator_invalid").to_string())
                            } else {
                                "••••••••••••••••".into()
                            }),
                    )
                    .child(
                        icon_button(
                            "generator-reveal",
                            if self.revealed { "eye-off" } else { "eye" },
                            "show",
                        )
                        .on_click(cx.listener(|this, _, _, cx| {
                            this.revealed = !this.revealed;
                            cx.notify();
                        })),
                    )
                    .child(
                        icon_button("generator-refresh", "dice-5", "ui.generate")
                            .disabled(!valid)
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.generate(cx);
                            })),
                    )
                    .child(
                        icon_button("generator-copy", "copy", "ui.copy")
                            .disabled(!valid)
                            .on_click(cx.listener(|this, _, _, cx| {
                                if this.editor.read(cx).connection().control.is_open()
                                    && let Some(value) = &this.generated
                                {
                                    super::clipboard::copy(value.expose().to_owned(), true, cx);
                                }
                            })),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!(
                        "{}: {:.1}",
                        tr("ui.entropy_bits"),
                        self.generated.as_ref().map_or(0., |s| s.entropy_bits())
                    )),
            )
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("password-mode")
                            .selected(!self.phrase)
                            .label(tr("password"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.phrase = false;
                                this.length
                                    .update(cx, |input, cx| input.set_value("30", window, cx));
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("phrase-mode")
                            .selected(self.phrase)
                            .label(tr("ui.passphrase"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.phrase = true;
                                this.length
                                    .update(cx, |input, cx| input.set_value("6", window, cx));
                                cx.notify();
                            })),
                    ),
            )
            .child(row(
                if self.phrase { "ui.words" } else { "ui.length" },
                field(&self.length, "ui.length"),
                cx,
            ))
            .when(!self.phrase, |el| {
                el.children(
                    ["ui.uppercase", "ui.lowercase", "ui.digits", "ui.symbols"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            Checkbox::new(("charset", index))
                                .label(tr(label))
                                .checked(self.sets[index])
                                .on_click(cx.listener(move |this, checked, _, cx| {
                                    this.sets[index] = *checked;
                                    this.generate(cx);
                                }))
                        }),
                )
                .child(row(
                    "ui.exclusions",
                    field(&self.exclusions, "ui.exclusions"),
                    cx,
                ))
            })
            .child(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("generator-close")
                            .label(tr("close"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        Button::new("generator-use")
                            .primary()
                            .disabled(!valid)
                            .label(tr("ui.use_value"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                if this.editor.read(cx).connection().control.is_open()
                                    && let Some(sample) = &this.generated
                                {
                                    this.editor.update(cx, |editor, cx| {
                                        editor.edit(|content| {
                                            content.password = sample.expose().to_owned()
                                        });
                                        cx.notify();
                                    });
                                    window.close_dialog(cx);
                                }
                            })),
                    ),
            )
    }
}
pub(super) fn open_generator(editor: Entity<EditorStore>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| GeneratorView::new(editor, window, cx));
    window.open_dialog(cx, move |dialog, window, _| {
        dialog
            .width(window.rem_size() * 36.25)
            .close_button(false)
            .child(view.clone())
            .footer(div())
    });
}
