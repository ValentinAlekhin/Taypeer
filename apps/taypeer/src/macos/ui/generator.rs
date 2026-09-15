//! Deterministic generator presentation; deliberately does not generate secrets.

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
    variant: usize,
    _subscriptions: Vec<Subscription>,
}
impl GeneratorView {
    fn new(editor: Entity<EditorStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let length = input("30", false, window, cx);
        let exclusions = input("", false, window, cx);
        let subscriptions = vec![
            cx.subscribe(&length, |_, _, _: &InputEvent, cx| cx.notify()),
            cx.subscribe(&exclusions, |_, _, _: &InputEvent, cx| cx.notify()),
        ];
        Self {
            editor,
            length,
            exclusions,
            phrase: false,
            sets: [true; 4],
            revealed: true,
            variant: 0,
            _subscriptions: subscriptions,
        }
    }
    fn sample(&self, cx: &App) -> Option<String> {
        let length = self.length.read(cx).value().parse::<usize>().ok()?;
        if self.phrase {
            if !(3..=20).contains(&length) {
                return None;
            }
            let words = [
                "public", "sample", "river", "copper", "window", "garden", "paper", "demo",
            ];
            return Some(
                (0..length)
                    .map(|i| words[(i + self.variant) % words.len()])
                    .collect::<Vec<_>>()
                    .join("-"),
            );
        }
        if !(1..=256).contains(&length) {
            return None;
        }
        let exclusions = self.exclusions.read(cx).value();
        let chars: Vec<_> = ["PUBLICDEMO", "publicdemo", "0123456789", "!@#$%"]
            .iter()
            .zip(self.sets)
            .filter(|(_, enabled)| *enabled)
            .flat_map(|(chars, _)| chars.chars())
            .filter(|c| !exclusions.contains(*c))
            .collect();
        if chars.is_empty() {
            return None;
        }
        Some(
            (0..length)
                .map(|i| chars[(i + self.variant) % chars.len()])
                .collect(),
        )
    }
}
impl Render for GeneratorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sample = self.sample(cx);
        let copy = sample.clone();
        let valid = sample.is_some();
        v_flex()
            .gap_4()
            .text_sm()
            .child(
                h_flex()
                    .gap_2()
                    .child(div().flex_1().font_family("Menlo").child(if self.revealed {
                        sample
                            .clone()
                            .unwrap_or_else(|| tr("ui.generator_invalid").to_string())
                    } else {
                        "••••••••••••••••".into()
                    }))
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
                                this.variant += 1;
                                cx.notify();
                            })),
                    )
                    .child(
                        icon_button("generator-copy", "copy", "ui.copy")
                            .disabled(!valid)
                            .on_click(move |_, _, cx| {
                                if let Some(copy) = &copy {
                                    cx.write_to_clipboard(ClipboardItem::new_string(copy.clone()));
                                }
                            }),
                    ),
            )
            .child(
                div()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ui.generator_sample")),
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
                                    cx.notify();
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
                                if let Some(sample) = &sample {
                                    this.editor.update(cx, |editor, cx| {
                                        editor.edit(|content| content.password = sample.clone());
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
    window.open_dialog(cx, move |dialog, _, _| {
        dialog
            .width(px(580.))
            .close_button(false)
            .child(view.clone())
            .footer(div())
    });
}
