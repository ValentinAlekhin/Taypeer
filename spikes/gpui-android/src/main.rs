use gpui_kit::assets::Assets;
use gpui_kit::component::{
    button::*,
    input::{Input, InputEvent, InputState},
    *,
};
use gpui_kit::*;
use taypeer_ui_probe::{ProbeSession, theme_palette};
use std::sync::Arc;

pub struct Example {
    russian: bool,
    input_state: Entity<InputState>,
    display_text: SharedString,
    session: Arc<ProbeSession>,
    revealed: SharedString,
    theme_choice: u8,
    font_size: u8,

    /// We need to keep the subscriptions alive with the Example entity.
    ///
    /// So if the Example entity is dropped, the subscriptions are also dropped.
    /// This is important to avoid memory leaks.
    _subscriptions: Vec<Subscription>,
}

impl Example {
    fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Synthetic password / Тестовый пароль")
                .masked(true)
        });

        let mut _subscriptions = vec![cx.subscribe_in(&input_state, window, {
            let input_state = input_state.clone();
            move |this, _, ev: &InputEvent, _window, cx| {
                if let InputEvent::Change = ev {
                    let count = input_state.read(cx).value().chars().count();
                    this.display_text = format!("Characters / Символов: {}", count).into();
                    cx.notify()
                }
            }
        })];
        _subscriptions.push(cx.observe_window_activation(window, |this, window, cx| {
            if !window.is_window_active() {
                this.lock(window, cx);
            }
            this.apply_theme(window, cx);
        }));
        _subscriptions.push(cx.observe_window_appearance(window, |this, window, cx| {
            this.apply_theme(window, cx);
        }));

        let preferences = std::fs::read_to_string(Self::preferences_path()).unwrap_or_default();
        let mut values = preferences.split_whitespace();
        let russian = values.next() == Some("ru");
        let theme_choice = values
            .next()
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|v| *v <= 2)
            .unwrap_or(0);
        let font_size = values
            .next()
            .and_then(|v| v.parse::<u8>().ok())
            .filter(|v| [14, 16, 18].contains(v))
            .unwrap_or(16);
        let this = Self {
            russian,
            input_state,
            display_text: SharedString::default(),
            session: ProbeSession::new(),
            revealed: SharedString::default(),
            theme_choice,
            font_size,
            _subscriptions,
        };
        this.apply_theme(window, cx);
        this
    }

    fn preferences_path() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("target/ui-preferences.txt")
    }

    fn save_preferences(&self) {
        // Only UI choices; no input or session state is persisted by the probe.
        let _ = std::fs::write(
            Self::preferences_path(),
            format!(
                "{} {} {}",
                if self.russian { "ru" } else { "en" },
                self.theme_choice,
                self.font_size
            ),
        );
    }

    fn apply_theme(&self, window: &mut Window, cx: &mut App) {
        let dark = match self.theme_choice {
            1 => false,
            2 => true,
            _ => matches!(
                window.appearance(),
                WindowAppearance::Dark | WindowAppearance::VibrantDark
            ),
        };
        Theme::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );
        let palette = theme_palette(dark).expect("embedded probe palette");
        let color = |role: &str| -> Hsla { rgb(palette[role]).into() };
        let theme = Theme::global_mut(cx);
        theme.background = color("background");
        theme.tokens.background = color("background").into();
        theme.foreground = color("foreground");
        theme.border = color("border");
        theme.input = color("border");
        theme.primary = color("primary");
        theme.primary_foreground = color("on_primary");
        theme.selection = color("selection");
        theme.ring = color("focus");
        theme.muted_foreground = color("muted");
        theme.danger = color("danger");
        theme.warning = color("warning");
        theme.success = color("success");
        theme.font_size = px(self.font_size as f32);
        Theme::sync_base(cx);
        window.refresh();
    }

    fn clear_input(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        // Replace the entire editor entity: set_value alone need not remove all editor caches.
        self.input_state = cx.new(|cx| {
            InputState::new(window, cx)
                .placeholder("Synthetic password / Тестовый пароль")
                .masked(true)
        });
        self._subscriptions[0] = cx.subscribe_in(
            &self.input_state,
            window,
            |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::Change) {
                    this.display_text = format!(
                        "Characters / Символов: {}",
                        this.input_state.read(cx).value().chars().count()
                    )
                    .into();
                    cx.notify();
                }
            },
        );
        self.display_text = SharedString::default();
    }

    fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let _ = self.session.lock();
        self.revealed = SharedString::default();
        self.clear_input(window, cx);
        window.close_all_dialogs(cx);
        cx.notify();
    }
}

impl Render for Example {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .p_5()
            .gap_2()
            .size_full()
            .items_center()
            .child(
                Button::new("language")
                    .label("English / Русский")
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.russian = !this.russian;
                        this.save_preferences();
                        cx.notify();
                    })),
            )
            .child(
                h_flex()
                    .gap_2()
                    .children(
                        [
                            (0, "System / Система"),
                            (1, "Light / Светлая"),
                            (2, "Dark / Тёмная"),
                        ]
                        .map(|(mode, label)| {
                            Button::new(("theme", mode as usize)).label(label).on_click(
                                cx.listener(move |this, _, window, cx| {
                                    this.theme_choice = mode;
                                    this.save_preferences();
                                    this.apply_theme(window, cx);
                                    cx.notify();
                                }),
                            )
                        }),
                    )
                    .children([14u8, 16, 18].map(|size| {
                        Button::new(("font", size as usize))
                            .label(size.to_string())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                this.font_size = size;
                                this.save_preferences();
                                this.apply_theme(window, cx);
                                cx.notify();
                            }))
                    })),
            )
            .child(if self.russian {
                "Только искусственные данные"
            } else {
                "Synthetic data only"
            })
            .child(
                div()
                    .id("entries")
                    .w_full()
                    .h_48()
                    .overflow_y_scroll()
                    .children((1..=100).map(|i| {
                        div().p_2().child(if self.russian {
                            format!("Тестовая запись {i}")
                        } else {
                            format!("Synthetic entry {i}")
                        })
                    })),
            )
            .child(Input::new(&self.input_state).mask_toggle())
            .child(self.display_text.clone())
            .child(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("open-sample")
                            .label("Send to Rust / В Rust")
                            .on_click(cx.listener(|this, _, window, cx| {
                                if this
                                    .session
                                    .open_sample(this.input_state.read(cx).value().to_string())
                                    .is_ok()
                                {
                                    this.revealed = "Sample in Rust / Пример в Rust".into();
                                    this.clear_input(window, cx);
                                }
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("reveal")
                            .label("Reveal / Показать")
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.revealed = match this
                                    .session
                                    .status()
                                    .and_then(|s| this.session.reveal(s.generation))
                                {
                                    Ok(reply) => reply.text.into(),
                                    Err(_) => "Locked / Заблокировано".into(),
                                };
                                cx.notify();
                            })),
                    )
                    .child(
                        Button::new("lock")
                            .label("Lock / Блокировка")
                            .on_click(cx.listener(|this, _, window, cx| this.lock(window, cx))),
                    ),
            )
            .child(self.revealed.clone())
            .child(
                Button::new("dialog")
                    .label(if self.russian {
                        "Диалог"
                    } else {
                        "Dialog"
                    })
                    .on_click(|_, window, cx| {
                        window.open_dialog(cx, |dialog, _, _| {
                            dialog
                                .title("Probe / Проверка")
                                .child("No data is saved / Данные не сохраняются")
                        });
                    }),
            )
            .children(Root::render_dialog_layer(window, cx))
    }
}

fn main() {
    let app = gpui_kit::application().with_assets(Assets);

    app.run(move |cx| {
        // This must be called before using any GPUI Component features.
        gpui_kit::init(cx);

        let window_options = WindowOptions {
            window_bounds: Some(WindowBounds::centered(size(px(800.), px(600.)), cx)),
            ..Default::default()
        };

        cx.spawn(async move |cx| {
            cx.open_window(window_options, |window, cx| {
                let view = cx.new(|cx| Example::new(window, cx));
                cx.activate(true);
                // This first level on the window, should be a Root.
                cx.new(|cx| Root::new(view, window, cx))
            })
            .expect("Failed to open window");
        })
        .detach();
    });
}
