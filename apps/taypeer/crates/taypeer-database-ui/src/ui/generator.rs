//! Explicit offline generation through the shared cryptographic generator.
use super::style::*;
use crate::ui_state::EditorStore;
use gpui_kit::{
    component::{
        Size,
        button::*,
        checkbox::Checkbox,
        input::{InputEvent, InputState, NumberInput},
        slider::{Slider, SliderEvent, SliderState},
        *,
    },
    prelude::FluentBuilder,
    *,
};
use taypeer_services::generator::GeneratedQuality;

#[derive(Clone, Copy, PartialEq, Eq)]
enum GeneratorMode {
    Password,
    Passphrase,
}
enum GeneratedOutput {
    Secret(taypeer_services::generator::GeneratedSecret),
    #[cfg(feature = "component-rendering")]
    PublicFixture(&'static str, f64, GeneratedQuality),
}
impl GeneratedOutput {
    fn value(&self) -> &str {
        match self {
            Self::Secret(value) => value.expose(),
            #[cfg(feature = "component-rendering")]
            Self::PublicFixture(value, _, _) => value,
        }
    }
    fn entropy(&self) -> f64 {
        match self {
            Self::Secret(value) => value.entropy_bits(),
            #[cfg(feature = "component-rendering")]
            Self::PublicFixture(_, entropy, _) => *entropy,
        }
    }
    fn quality(&self) -> GeneratedQuality {
        match self {
            Self::Secret(value) => value.quality(),
            #[cfg(feature = "component-rendering")]
            Self::PublicFixture(_, _, quality) => *quality,
        }
    }
}
struct GeneratorView {
    // Only the product view can insert or copy a generated secret.
    editor: Option<Entity<EditorStore>>,
    length: Entity<InputState>,
    words: Entity<InputState>,
    separator: Entity<InputState>,
    exclusions: Entity<InputState>,
    slider: Entity<SliderState>,
    mode: GeneratorMode,
    sets: [bool; 4],
    exclude_similar: bool,
    revealed: bool,
    generated: Option<GeneratedOutput>,
    _subscriptions: Vec<Subscription>,
}
fn command_label(button: Button, label: &str) -> Button {
    button.px(rems(0.625)).accessibility_label(tr(label)).child(
        div()
            .w_full()
            .text_size(rems(13. / 16.))
            .font_weight(FontWeight::NORMAL)
            .child(tr(label)),
    )
}
impl GeneratorView {
    fn new(
        editor: Option<Entity<EditorStore>>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let length = cx.new(|cx| InputState::new(window, cx).default_value("30").step(1.));
        let words = cx.new(|cx| InputState::new(window, cx).default_value("6").step(1.));
        let separator = input("-", false, window, cx);
        let exclusions = input("", false, window, cx);
        let slider = cx.new(|_| {
            SliderState::new()
                .min(1.)
                .max(256.)
                .step(1.)
                .default_value(30.)
        });
        let mut subscriptions = vec![
            cx.subscribe_in(
                &length,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::Change) {
                        if let Ok(length) = this.length.read(cx).value().parse::<u16>()
                            && (1..=256).contains(&length)
                        {
                            this.slider.update(cx, |slider, cx| {
                                slider.set_value(f32::from(length), window, cx)
                            });
                        }
                        this.generate(cx);
                    }
                },
            ),
            cx.subscribe_in(
                &slider,
                window,
                |this, _, event: &SliderEvent, window, cx| {
                    if let SliderEvent::Change(value) = event {
                        this.length.update(cx, |input, cx| {
                            input.set_value((value.end().round() as u16).to_string(), window, cx)
                        });
                        this.generate(cx);
                    }
                },
            ),
        ];
        for input in [&words, &separator, &exclusions] {
            subscriptions.push(cx.subscribe(input, |this, _, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.generate(cx);
                }
            }));
        }
        let mut view = Self {
            editor,
            length,
            words,
            separator,
            exclusions,
            slider,
            mode: GeneratorMode::Password,
            sets: [true; 4],
            exclude_similar: false,
            revealed: true,
            generated: None,
            _subscriptions: subscriptions,
        };
        if view.editor.is_some() {
            view.generate(cx);
        }
        view
    }
    fn generate(&mut self, cx: &mut Context<Self>) {
        #[cfg(feature = "component-rendering")]
        if self.editor.is_none() {
            cx.notify();
            return;
        }
        use taypeer_services::generator::{self, PasswordOptions};
        self.generated = match self.mode {
            GeneratorMode::Passphrase => {
                self.words.read(cx).value().parse().ok().and_then(|count| {
                    generator::passphrase(count, self.separator.read(cx).value().as_str()).ok()
                })
            }
            GeneratorMode::Password => {
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
                            exclude_similar: self.exclude_similar,
                            exclude: self.exclusions.read(cx).value().to_string(),
                        })
                        .ok()
                    })
            }
        }
        .map(GeneratedOutput::Secret);
        cx.notify();
    }
    fn options(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let owner = cx.entity().downgrade();
        let options = cx.new(|_| GeneratorOptions {
            owner,
            exclusions: self.exclusions.clone(),
            exclude_similar: self.exclude_similar,
        });
        window.open_dialog(cx, move |dialog, window, _| {
            dialog
                .title(tr("ui.generator_options"))
                .width(window.rem_size() * 35.)
                .child(options.clone())
                .footer(
                    default_button("generator-options-close")
                        .label(tr("close"))
                        .on_click(|_, window, cx| window.close_dialog(cx)),
                )
        });
    }
    fn insert(&self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = &self.editor else {
            return false;
        };
        let Some(GeneratedOutput::Secret(value)) = &self.generated else {
            return false;
        };
        if !editor.read(cx).connection().control.is_open() {
            return false;
        }
        editor.update(cx, |editor, cx| {
            editor.set_field(
                crate::ui_state::EntryField::Password,
                value.expose().to_owned(),
            );
            cx.notify();
        });
        true
    }
}
struct GeneratorOptions {
    owner: WeakEntity<GeneratorView>,
    exclusions: Entity<InputState>,
    exclude_similar: bool,
}
impl Render for GeneratorOptions {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .gap_4()
            .child(
                Checkbox::new("generator-exclude-similar")
                    .label(tr("ui.exclude_similar"))
                    .checked(self.exclude_similar)
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.exclude_similar = *checked;
                        let _ = this.owner.update(cx, |owner, cx| {
                            owner.exclude_similar = *checked;
                            owner.generate(cx);
                        });
                        cx.notify();
                    })),
            )
            .child(row(
                "ui.exclusions",
                field(&self.exclusions, "ui.exclusions"),
                cx,
            ))
    }
}
impl Render for GeneratorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let valid = self.generated.is_some();
        let entropy = self.generated.as_ref().map_or(0., GeneratedOutput::entropy);
        let characters = self
            .generated
            .as_ref()
            .map_or(0, |value| value.value().chars().count());
        let (quality, color) = match self
            .generated
            .as_ref()
            .map_or(GeneratedQuality::Low, GeneratedOutput::quality)
        {
            GeneratedQuality::High => ("ui.quality_high", cx.theme().success),
            GeneratedQuality::Medium => ("ui.quality_medium", cx.theme().warning),
            GeneratedQuality::Low => ("ui.quality_low", cx.theme().danger),
        };
        let count = tr("ui.character_count").replace("%{count}", &characters.to_string());
        let bits = tr("ui.entropy_bits").replace(
            "%{bits}",
            &format!("{entropy:.2}").replace('.', tr("ui.decimal_separator").as_ref()),
        );
        v_flex()
            .id("generator-panel")
            .test_support()
            .gap_6()
            .text_sm()
            .font_weight(FontWeight::NORMAL)
            .capture_key_down(|_, _, cx| taypeer_desktop_platform::activity(cx))
            .capture_any_mouse_down(|_, _, cx| taypeer_desktop_platform::activity(cx))
            .child(
                v_flex()
                    .gap_3()
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                h_flex()
                                    .id("generator-result")
                                    .test_support()
                                    .w(rems(if self.mode == GeneratorMode::Passphrase {
                                        38.
                                    } else {
                                        26.
                                    }))
                                    .min_w_0()
                                    .h(rems(if self.mode == GeneratorMode::Passphrase {
                                        2.5
                                    } else {
                                        2.25
                                    }))
                                    .px_3()
                                    .rounded(rems(0.25))
                                    .bg(cx.theme().title_bar)
                                    .child(
                                        div()
                                            .flex_1()
                                            .min_w_0()
                                            .truncate()
                                            .font_family(cx.theme().mono_font_family.clone())
                                            .child(if !valid {
                                                tr("ui.generator_invalid").to_string()
                                            } else if self.revealed {
                                                self.generated
                                                    .as_ref()
                                                    .map_or(String::new(), |value| {
                                                        value.value().to_owned()
                                                    })
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
                                        .on_click(
                                            cx.listener(|this, _, _, cx| {
                                                this.revealed = !this.revealed;
                                                cx.notify();
                                            }),
                                        ),
                                    ),
                            )
                            .child(
                                icon_button("generator-refresh", "dice-5", "ui.generate")
                                    .on_click(cx.listener(|this, _, _, cx| this.generate(cx))),
                            )
                            .child(
                                icon_button("generator-copy", "copy", "ui.copy")
                                    .ml_2()
                                    .disabled(!valid)
                                    .on_click(cx.listener(|this, _, _, cx| {
                                        if let Some(editor) = &this.editor
                                            && editor.read(cx).connection().control.is_open()
                                            && let Some(GeneratedOutput::Secret(value)) =
                                                &this.generated
                                        {
                                            super::clipboard::copy(
                                                value.expose().to_owned(),
                                                true,
                                                cx,
                                            );
                                        }
                                    })),
                            ),
                    )
                    .child(
                        div()
                            .id("generator-quality-bar")
                            .test_support()
                            .h(px(3.))
                            .rounded(px(1.))
                            .bg(color),
                    )
                    .child(
                        h_flex()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child(
                                div()
                                    .id("generator-quality")
                                    .test_support()
                                    .flex_1()
                                    .child(tr(quality)),
                            )
                            .child(
                                div()
                                    .id("generator-count")
                                    .test_support()
                                    .aria_label(count.clone())
                                    .flex_1()
                                    .text_center()
                                    .child(count),
                            )
                            .child(
                                div()
                                    .id("generator-entropy")
                                    .test_support()
                                    .aria_label(bits.clone())
                                    .flex_1()
                                    .text_right()
                                    .child(bits),
                            ),
                    ),
            )
            .child(
                tabs(
                    "generator-tabs",
                    &["password", "ui.passphrase"],
                    usize::from(self.mode == GeneratorMode::Passphrase),
                    cx,
                )
                .on_click(cx.listener(|this, index, window, cx| {
                    this.mode = if *index == 0 {
                        GeneratorMode::Password
                    } else {
                        GeneratorMode::Passphrase
                    };
                    this.generate(cx);
                    // Rebuild the outer dialog width when the retained mode changes.
                    window.refresh();
                })),
            )
            .when(self.mode == GeneratorMode::Password, |el| {
                el.child(
                    v_flex()
                        .child(
                            div()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("ui.generator_length")),
                        )
                        .child(
                            h_flex()
                                .mt(rems(0.125))
                                .gap_6()
                                .child(
                                    div()
                                        .id("generator-length-slider")
                                        .test_support()
                                        .flex_1()
                                        .child(Slider::new(&self.slider).h(rems(2.25))),
                                )
                                .child(
                                    div()
                                        .id("generator-length")
                                        .test_support()
                                        .w(rems(8.5))
                                        .child(
                                            NumberInput::new(&self.length)
                                                .with_size(Size::Small)
                                                .bg(cx.theme().title_bar)
                                                .h(rems(2.25))
                                                .border_0()
                                                .rounded(rems(0.25)),
                                        ),
                                ),
                        )
                        .child(
                            div()
                                .mt_6()
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("ui.character_sets")),
                        )
                        .child(
                            h_flex().mt_2().gap_3().children(
                                [
                                    ("A–Z", "ui.uppercase"),
                                    ("a–z", "ui.lowercase"),
                                    ("0–9", "ui.digits"),
                                    ("!@#", "ui.symbols"),
                                ]
                                .into_iter()
                                .enumerate()
                                .map(
                                    |(index, (label, accessible))| {
                                        div()
                                            .w(rems(6.))
                                            .h_8()
                                            .px_3()
                                            .rounded(rems(0.25))
                                            .when(self.sets[index], |cell| {
                                                cell.bg(cx.theme().list_active)
                                            })
                                            .child(
                                                Checkbox::new(("charset", index))
                                                    .text_size(rems(13. / 16.))
                                                    .label(label)
                                                    .checked(self.sets[index])
                                                    .on_click(cx.listener(
                                                        move |this, checked, _, cx| {
                                                            this.sets[index] = *checked;
                                                            this.generate(cx);
                                                        },
                                                    ))
                                                    .aria_label(tr(accessible)),
                                            )
                                    },
                                ),
                            ),
                        ),
                )
            })
            .when(self.mode == GeneratorMode::Passphrase, |el| {
                el.child(
                    v_flex()
                        .mt(rems(0.3125))
                        .child(
                            taypeer_ui::forms::form_row(
                                "ui.words",
                                Some(self.words.focus_handle(cx)),
                                div().id("generator-words").test_support().child(
                                    NumberInput::new(&self.words)
                                        .with_size(Size::Small)
                                        .h(rems(2.))
                                        .bg(cx.theme().title_bar)
                                        .border_0(),
                                ),
                                cx,
                            )
                            .min_h(rems(3.)),
                        )
                        .child(
                            taypeer_ui::forms::form_row(
                                "ui.separator",
                                Some(self.separator.focus_handle(cx)),
                                field(&self.separator, "ui.separator"),
                                cx,
                            )
                            .min_h(rems(3.)),
                        )
                        .child(
                            div()
                                .ml(rems(10.))
                                .mt(rems(0.125))
                                .h(rems(1.25))
                                .text_xs()
                                .text_color(cx.theme().muted_foreground)
                                .child(tr("ui.wordlist_hint")),
                        ),
                )
            })
            .child(
                h_flex()
                    .mt_5()
                    .when(self.mode == GeneratorMode::Passphrase, |footer| {
                        footer
                            .mt_4()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .pt(rems(0.9375))
                    })
                    .id("generator-actions")
                    .test_support()
                    .gap_3()
                    .when(self.mode == GeneratorMode::Password, |el| {
                        el.child(
                            command_label(
                                default_button("generator-options"),
                                "ui.generator_advanced",
                            )
                            .w(rems(11.375))
                            .on_click(cx.listener(|this, _, window, cx| this.options(window, cx))),
                        )
                    })
                    .child(div().flex_1())
                    .child(
                        command_label(default_button("generator-close"), "close")
                            .w(rems(if self.mode == GeneratorMode::Passphrase {
                                6.5
                            } else {
                                6.25
                            }))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(
                        command_label(Button::new("generator-use"), "ui.use_value")
                            .w(rems(if self.mode == GeneratorMode::Passphrase {
                                11.5
                            } else {
                                7.5
                            }))
                            .primary()
                            .disabled(!valid)
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this.insert(cx) {
                                    window.close_dialog(cx);
                                }
                            })),
                    ),
            )
    }
}
pub(super) fn open_generator(editor: Entity<EditorStore>, window: &mut Window, cx: &mut App) {
    let view = cx.new(|cx| GeneratorView::new(Some(editor), window, cx));
    window.open_dialog(cx, move |dialog, window, cx| {
        dialog
            .width(
                window.rem_size()
                    * if view.read(cx).mode == GeneratorMode::Passphrase {
                        46.5
                    } else {
                        35.
                    },
            )
            .p_6()
            .close_button(false)
            .child(view.clone())
            .footer(div())
    });
}

#[cfg(feature = "component-rendering")]
#[derive(Clone, Copy)]
/// Public deterministic layouts using the product generator view.
pub enum GeneratorPreviewCase {
    /// Thirty characters and known entropy from the public fixture.
    Password,
    /// Six public example words with the product separator controls.
    Passphrase,
}
#[cfg(feature = "component-rendering")]
/// Create a retained public fixture; it neither generates nor copies real secrets.
pub fn generator_preview(case: GeneratorPreviewCase, window: &mut Window, cx: &mut App) -> AnyView {
    struct Preview(Entity<GeneratorView>);
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().p_6().child(self.0.clone())
        }
    }
    let view = cx.new(|cx| {
        let mut view = GeneratorView::new(None, window, cx);
        view.mode = match case {
            GeneratorPreviewCase::Password => GeneratorMode::Password,
            GeneratorPreviewCase::Passphrase => GeneratorMode::Passphrase,
        };
        view.generated = Some(match case {
            GeneratorPreviewCase::Password => GeneratedOutput::PublicFixture(
                "mT7!qR4#vN9@kL2$wH6%pD8&xC3?zB",
                196.63766555032913,
                GeneratedQuality::High,
            ),
            GeneratorPreviewCase::Passphrase => GeneratedOutput::PublicFixture(
                "river-candle-paper-meadow-copper-sunset",
                77.54887502163469,
                GeneratedQuality::High,
            ),
        });
        view
    });
    cx.new(|_| Preview(view)).into()
}
