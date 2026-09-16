//! Device preferences and authenticated shared database settings.

use super::{appearance::PreferencesStore, forms, style::*, workspace::WorkspaceStore};
use crate::preferences::{FONT_SIZES, Language, ThemePreference};
use crate::ui_state::FormError;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        menu::{DropdownMenu, PopupMenuItem},
        *,
    },
    *,
};

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Device,
    Database,
}
pub(super) struct SettingsView {
    store: Entity<WorkspaceStore>,
    preferences: Entity<PreferencesStore>,
    tab: SettingsTab,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl SettingsView {
    pub fn new(
        store: Entity<WorkspaceStore>,
        preferences: Entity<PreferencesStore>,
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
                    .label(self.store.read(cx).local().device_name.clone())
                    .disabled(self.store.read(cx).settings_busy())
                    .on_click(cx.listener(|this, _, window, cx| {
                        let store = this.store.clone();
                        let initial = store.read(cx).local().device_name.clone();
                        forms::text_form(
                            "ui.device_name",
                            vec![("name", initial, false)],
                            Box::new(move |values, _, cx| {
                                crate::ui_state::require_name(&values[0])?;
                                let mut candidate = store.read(cx).local().clone();
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
                                    self.store.read(cx).idle_seconds() == [60, 300, 900][index],
                                )
                                .disabled(self.store.read(cx).settings_busy())
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
                                    self.store.read(cx).local().clipboard_seconds
                                        == [None, Some(30), Some(60)][index],
                                )
                                .disabled(self.store.read(cx).settings_busy())
                                .label(tr(label))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    let mut local = this.store.read(cx).local().clone();
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
                                self.store.read(cx).idle_seconds()
                            ))
                            .disabled(self.store.read(cx).settings_busy())
                            .on_click(
                                cx.listener(|this, _, window, cx| this.interval(false, window, cx)),
                            ),
                    )
                    .child(
                        Button::new("custom-clipboard")
                            .label(tr("ui.clipboard_timeout"))
                            .disabled(self.store.read(cx).settings_busy())
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
                Button::new("relay")
                    .disabled(true)
                    .label(tr("ui.network_unavailable")),
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
            self.store.read(cx).local().clipboard_seconds.unwrap_or(30)
        } else {
            self.store.read(cx).idle_seconds()
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
                            let mut local = store.local().clone();
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
    fn database(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        if !state.state().is_unlocked() {
            return empty("ui.settings_locked", cx);
        }
        let Some(id) = state.state().database.as_ref() else {
            return empty("ui.settings_locked", cx);
        };
        let Some(database) = state.catalog().read(cx).database(id) else {
            return empty("ui.settings_locked", cx);
        };
        let id = id.clone();
        let policy = database.policy;
        v_flex()
            .gap_3()
            .p_6()
            .max_w(rems(58.))
            .child(row("name", database.name.clone(), cx))
            .child(row(
                "ui.description",
                database.description.clone().unwrap_or_default(),
                cx,
            ))
            .when(database.metadata_conflict, |el| {
                el.child(
                    div()
                        .px_6()
                        .text_color(cx.theme().danger)
                        .child(tr("ui.metadata_conflict")),
                )
            })
            .child(row(
                "ui.storage",
                database.path.to_string_lossy().into_owned(),
                cx,
            ))
            .child(row(
                "ui.managing_device",
                tr(if database.managing {
                    "ui.this_device"
                } else {
                    "ui.another_device"
                }),
                cx,
            ))
            .child(
                h_flex()
                    .px_6()
                    .gap_2()
                    .child(
                        Button::new("database-info")
                            .label(tr("ui.database_info"))
                            .disabled(!database.writable || database.metadata_conflict)
                            .on_click(cx.listener(move |this, _, window, cx| {
                                forms::database(this.store.clone(), Some(id.clone()), window, cx)
                            })),
                    )
                    .child(
                        Button::new("protection")
                            .label(tr("ui.protection"))
                            .disabled(!database.managing)
                            .on_click(
                                cx.listener(|this, _, window, cx| this.protection(window, cx)),
                            ),
                    ),
            )
            .child(row(
                "ui.kdf_seconds",
                format!("{:.3}", f64::from(policy.kdf_target_ms()) / 1000.),
                cx,
            ))
            .child(row(
                "ui.attachment_limit",
                format!("{} MiB", policy.attachment_bytes() / (1024 * 1024)),
                cx,
            ))
            .child(row(
                "ui.total_attachment_limit",
                format!("{} MiB", policy.total_attachment_bytes() / (1024 * 1024)),
                cx,
            ))
            .into_any_element()
    }
    fn protection(&self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.store.read(cx);
        let Some(connection) = state.connection().cloned() else {
            return;
        };
        let Some(database) = state.catalog().read(cx).database(&connection.database) else {
            return;
        };
        let policy = database.policy;
        let Ok(operation) = taypeer_services::new_operation_id()
            .map(|id| taypeer_trust::Digest::of(id.as_str().as_bytes()))
        else {
            return;
        };
        let store = self.store.clone();
        forms::text_form(
            "ui.protection",
            vec![
                (
                    "ui.kdf_seconds",
                    format!("{:.3}", f64::from(policy.kdf_target_ms()) / 1000.),
                    false,
                ),
                (
                    "ui.attachment_limit",
                    (policy.attachment_bytes() / (1024 * 1024)).to_string(),
                    false,
                ),
                (
                    "ui.total_attachment_limit",
                    (policy.total_attachment_bytes() / (1024 * 1024)).to_string(),
                    false,
                ),
                ("password", String::new(), true),
            ],
            Box::new(move |values, _, _| {
                let seconds: f64 = values[0].parse().map_err(|_| FormError::InvalidNumber)?;
                if !seconds.is_finite() || !(0.5..=5.).contains(&seconds) {
                    return Err(FormError::InvalidNumber);
                }
                let bytes = |value: &str| {
                    value
                        .parse::<u64>()
                        .ok()
                        .and_then(|n| n.checked_mul(1024 * 1024))
                        .ok_or(FormError::InvalidNumber)
                };
                let updated = taypeer_core::DatabasePolicy::new(
                    bytes(&values[1])?,
                    bytes(&values[2])?,
                    (seconds * 1000.).round() as u32,
                )
                .map_err(|_| FormError::InvalidNumber)?;
                let password = (policy.kdf_target_ms() != updated.kdf_target_ms())
                    .then(|| zeroize::Zeroizing::new(values[3].as_bytes().to_vec()));
                let ticket = connection.command::<serde_json::Value>(
                    taypeer_runtime::Command::SetDatabasePolicy {
                        operation,
                        policy: updated,
                        password,
                    },
                );
                let store = store.clone();
                Ok(Some(Box::new(move |done, _, cx| {
                    store.update(cx, |store, _| {
                        store.watch(ticket, move |store, result, window, cx| {
                            let result = result.map(|_| ()).map_err(FormError::Runtime);
                            if result.is_ok() {
                                store.refresh(cx);
                            }
                            done(result, window, cx);
                        })
                    });
                })))
            }),
            window,
            cx,
        );
    }
}
impl Render for SettingsView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let content = if self.tab == SettingsTab::Device {
            self.device(cx)
        } else {
            self.database(cx)
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
