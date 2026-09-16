//! Device preferences and authenticated shared database settings.

use crate::preferences::{FONT_SIZES, Language, ThemePreference};
use crate::{appearance::PreferencesStore, host::SettingsHost};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        menu::{DropdownMenu, PopupMenuItem},
        *,
    },
    *,
};
use taypeer_ui::FormError;
use taypeer_ui::{forms, style::*};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Device,
    Database,
}
/// Device settings presentation; the database feature supplies its own content slot.
pub struct SettingsView<H: SettingsHost> {
    store: Entity<H>,
    preferences: Entity<PreferencesStore>,
    tab: SettingsTab,
    scroll: ScrollHandle,
    database: AnyView,
    _subscriptions: Vec<Subscription>,
}
impl<H: SettingsHost> SettingsView<H> {
    /// Retain settings controls and subscriptions; the host owns requested changes.
    pub fn new(
        store: Entity<H>,
        preferences: Entity<PreferencesStore>,
        database: AnyView,
        _window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&preferences, |_, _, cx| cx.notify()),
        ];
        Self {
            store,
            preferences,
            tab: SettingsTab::Device,
            database,
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }
    fn device(&self, cx: &mut Context<Self>) -> AnyElement {
        let prefs = self.preferences.read(cx).values();
        let language = prefs.language;
        let theme = prefs.theme;
        let size = prefs.font_size;
        let languages = self.preferences.clone();
        let themes = self.preferences.clone();
        let sizes = self.preferences.clone();
        v_flex()
            .gap_3()
            .p_6()
            .max_w(rems(58.))
            .child(row(
                "ui.device_name",
                Button::new("device-name")
                    .ghost()
                    .label(self.store.read(cx).local(cx).device_name.clone())
                    .disabled(self.store.read(cx).settings_busy(cx))
                    .on_click(cx.listener(|this, _, window, cx| {
                        let store = this.store.clone();
                        let initial = store.read(cx).local(cx).device_name.clone();
                        forms::text_form(
                            "ui.device_name",
                            vec![("name", initial, false)],
                            Box::new(move |values, _, cx| {
                                taypeer_ui::require_name(&values[0])?;
                                let mut candidate = store.read(cx).local(cx).clone();
                                candidate.device_name = values[0].clone();
                                let store = store.clone();
                                Ok(Some(Box::new(move |done, window, cx| {
                                    store.update(cx, |store, cx| {
                                        store.save_local_form(candidate, done, window, cx)
                                    })
                                })))
                            }),
                            window,
                            cx,
                        );
                    })),
                cx,
            ))
            .child(row(
                "language",
                Button::new("language")
                    .ghost()
                    .justify_between()
                    .w_full()
                    .label(language.label())
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for language in Language::ALL {
                            let target = languages.clone();
                            menu = menu.item(PopupMenuItem::new(language.label()).on_click(
                                move |_, window, cx| {
                                    target.update(cx, |prefs, cx| {
                                        prefs.set_language(language, window, cx)
                                    })
                                },
                            ));
                        }
                        menu
                    }),
                cx,
            ))
            .child(row(
                "theme",
                Button::new("theme")
                    .ghost()
                    .justify_between()
                    .w_full()
                    .label(tr(theme.key()))
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for theme in ThemePreference::ALL {
                            let target = themes.clone();
                            menu = menu.item(PopupMenuItem::new(tr(theme.key())).on_click(
                                move |_, window, cx| {
                                    target
                                        .update(cx, |prefs, cx| prefs.set_theme(theme, window, cx))
                                },
                            ));
                        }
                        menu
                    }),
                cx,
            ))
            .child(row(
                "font",
                Button::new("font-size")
                    .ghost()
                    .justify_between()
                    .w_full()
                    .label(size.to_string())
                    .dropdown_caret(true)
                    .dropdown_menu(move |mut menu, _, _| {
                        for size in FONT_SIZES {
                            let target = sizes.clone();
                            menu = menu.item(PopupMenuItem::new(size.to_string()).on_click(
                                move |_, window, cx| {
                                    target.update(cx, |prefs, cx| prefs.set_font(size, window, cx))
                                },
                            ));
                        }
                        menu
                    }),
                cx,
            ))
            .child(row(
                "ui.auto_lock",
                h_flex().gap_2().children(
                    ["ui.minute", "ui.five_minutes", "ui.fifteen_minutes"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            Button::new(("autolock", index))
                                .ghost()
                                .selected(
                                    self.store.read(cx).idle_seconds(cx) == [60, 300, 900][index],
                                )
                                .disabled(self.store.read(cx).settings_busy(cx))
                                .label(tr(label))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.store.update(cx, |store, cx| {
                                        store.set_idle([60, 300, 900][index], cx)
                                    });
                                }))
                        }),
                ),
                cx,
            ))
            .child(row(
                "ui.clipboard_timeout",
                h_flex().gap_2().children(
                    ["ui.never", "ui.thirty_seconds", "ui.minute"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            Button::new(("clipboard", index))
                                .ghost()
                                .selected(
                                    self.store.read(cx).local(cx).clipboard_seconds
                                        == [None, Some(30), Some(60)][index],
                                )
                                .disabled(self.store.read(cx).settings_busy(cx))
                                .label(tr(label))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut local = this.store.read(cx).local(cx).clone();
                                    local.clipboard_seconds = [None, Some(30), Some(60)][index];
                                    this.store
                                        .update(cx, |store, cx| store.save_local(local, cx));
                                }))
                        }),
                ),
                cx,
            ))
            .child(row(
                "ui.custom_intervals",
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("custom-idle")
                            .label(format!(
                                "{}: {} s",
                                tr("ui.auto_lock"),
                                self.store.read(cx).idle_seconds(cx)
                            ))
                            .disabled(self.store.read(cx).settings_busy(cx))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.interval(false, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("custom-clipboard")
                            .label(tr("ui.clipboard_timeout"))
                            .disabled(self.store.read(cx).settings_busy(cx))
                            .on_click(
                                cx.listener(|this, _, window, cx| this.interval(true, window, cx)),
                            ),
                    ),
                cx,
            ))
            .child(section("ui.touch_id"))
            .child(row(
                "ui.biometric",
                Button::new("biometric")
                    .disabled(true)
                    .label(tr("ui.touch_id_unavailable")),
                cx,
            ))
            .child(section("ui.connection"))
            .child(row(
                "ui.relay",
                h_flex()
                    .gap_2()
                    .children(
                        [
                            (
                                "sync.relay_public",
                                crate::local_settings::RelayPreference::Public,
                            ),
                            (
                                "sync.relay_off",
                                crate::local_settings::RelayPreference::Disabled,
                            ),
                        ]
                        .into_iter()
                        .map(|(label, choice)| {
                            Button::new(label)
                                .ghost()
                                .label(tr(label))
                                .selected(self.store.read(cx).local(cx).relay == choice)
                                .disabled(
                                    self.store.read(cx).settings_busy(cx)
                                        || self.store.read(cx).exchange_busy(cx),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.store
                                        .update(cx, |s, cx| s.set_relay(choice.clone(), None, cx))
                                }))
                        }),
                    )
                    .child(
                        Button::new("custom-relay")
                            .ghost()
                            .label(tr("sync.relay_custom"))
                            .selected(matches!(
                                self.store.read(cx).local(cx).relay,
                                crate::local_settings::RelayPreference::Custom(_)
                            ))
                            .disabled(
                                self.store.read(cx).settings_busy(cx)
                                    || self.store.read(cx).exchange_busy(cx),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                let value = match &this.store.read(cx).local(cx).relay {
                                    crate::local_settings::RelayPreference::Custom(url) => {
                                        url.clone()
                                    }
                                    _ => String::new(),
                                };
                                let store = this.store.clone();
                                forms::text_form(
                                    "sync.relay_custom",
                                    vec![("sync.relay_url", value, false)],
                                    Box::new(move |values, _, _| {
                                        let relay = crate::local_settings::RelayPreference::Custom(
                                            values[0].clone(),
                                        );
                                        relay.setting().map_err(FormError::Runtime)?;
                                        let store = store.clone();
                                        Ok(Some(Box::new(move |done, _, cx| {
                                            store.update(cx, |s, cx| {
                                                s.set_relay(relay, Some(done), cx)
                                            })
                                        })))
                                    }),
                                    window,
                                    cx,
                                );
                            })),
                    ),
                cx,
            ))
            .child(
                div()
                    .px_6()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ui.local_settings_hint")),
            )
            .when_some(self.preferences.read(cx).error(), |el, error| {
                el.child(div().p_4().text_color(cx.theme().danger).child(tr(error)))
            })
            .into_any_element()
    }
    fn interval(&self, clipboard: bool, window: &mut Window, cx: &mut Context<Self>) {
        let initial = if clipboard {
            self.store
                .read(cx)
                .local(cx)
                .clipboard_seconds
                .unwrap_or(30)
        } else {
            self.store.read(cx).idle_seconds(cx)
        };
        let store = self.store.clone();
        forms::text_form(
            if clipboard {
                "ui.clipboard_timeout"
            } else {
                "ui.auto_lock"
            },
            vec![("ui.seconds", initial.to_string(), false)],
            Box::new(move |values, _, _| {
                let seconds = values[0]
                    .parse::<u32>()
                    .ok()
                    .filter(|n| *n > 0)
                    .ok_or(FormError::InvalidNumber)?;
                let store = store.clone();
                Ok(Some(Box::new(move |done, window, cx| {
                    store.update(cx, |store, cx| {
                        if clipboard {
                            let mut local = store.local(cx).clone();
                            local.clipboard_seconds = Some(seconds);
                            store.save_local_form(local, done, window, cx);
                        } else {
                            store.set_idle_form(seconds, done, window, cx);
                        }
                    })
                })))
            }),
            window,
            cx,
        );
    }
}
impl<H: SettingsHost> Render for SettingsView<H> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.tab == SettingsTab::Device {
            self.device(cx)
        } else {
            self.database.clone().into_any_element()
        };
        v_flex()
            .size_full()
            .child(
                h_flex()
                    .h(rems(2.75))
                    .px_6()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .flex_1()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("settings")),
                    )
                    .child(
                        icon_button("close-settings", "x", "close").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.settings(cx);
                                })
                            },
                        )),
                    ),
            )
            .child(
                tabs(
                    "settings-tabs",
                    &["ui.device", "ui.database"],
                    usize::from(self.tab == SettingsTab::Database),
                    cx,
                )
                .on_click(cx.listener(|this, index, _, cx| {
                    this.tab = if *index == 0 {
                        SettingsTab::Device
                    } else {
                        SettingsTab::Database
                    };
                    cx.notify();
                })),
            )
            .child(
                div()
                    .id("settings-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(content),
            )
    }
}
