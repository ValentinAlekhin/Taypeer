//! Device appearance and editable synthetic settings, with separate lifetimes.

use super::{appearance::PreferencesStore, forms, style::*, workspace::WorkspaceStore};
use crate::preferences::{FONT_SIZES, Language, ThemePreference};
use crate::ui_state::{DatabaseId, FormError};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::InputState,
        menu::{DropdownMenu, PopupMenuItem},
        switch::Switch,
        *,
    },
    *,
};
use std::collections::BTreeMap;

#[derive(Clone, Copy, PartialEq, Eq)]
enum SettingsTab {
    Device,
    Database,
}
pub(super) struct SettingsView {
    store: Entity<WorkspaceStore>,
    preferences: Entity<PreferencesStore>,
    device_name: Entity<InputState>,
    tab: SettingsTab,
    auto_lock: usize,
    clipboard: usize,
    biometric: bool,
    relay: bool,
    protection: BTreeMap<DatabaseId, String>,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl SettingsView {
    pub fn new(
        store: Entity<WorkspaceStore>,
        preferences: Entity<PreferencesStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let device_name = input("MacBook Pro", false, window, cx);
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe(&preferences, |_, _, cx| cx.notify()),
        ];
        Self {
            store,
            preferences,
            device_name,
            tab: SettingsTab::Device,
            auto_lock: 2,
            clipboard: 1,
            biometric: false,
            relay: true,
            protection: BTreeMap::new(),
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
                field(&self.device_name, "ui.device_name"),
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
                    ["ui.never", "ui.minute", "ui.five_minutes"]
                        .into_iter()
                        .enumerate()
                        .map(|(index, label)| {
                            Button::new(("autolock", index))
                                .ghost()
                                .selected(self.auto_lock == index)
                                .label(tr(label))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.auto_lock = index;
                                    cx.notify();
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
                                .selected(self.clipboard == index)
                                .label(tr(label))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.clipboard = index;
                                    cx.notify();
                                }))
                        }),
                ),
                cx,
            ))
            .child(section("ui.touch_id"))
            .child(row(
                "ui.biometric",
                Switch::new("biometric")
                    .checked(self.biometric)
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.biometric = *checked;
                        cx.notify();
                    })),
                cx,
            ))
            .child(section("ui.connection"))
            .child(row(
                "ui.relay",
                Switch::new("relay")
                    .checked(self.relay)
                    .on_click(cx.listener(|this, checked, _, cx| {
                        this.relay = *checked;
                        cx.notify();
                    })),
                cx,
            ))
            .child(
                div()
                    .px_6()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ui.settings_sample")),
            )
            .when_some(self.preferences.read(cx).error(), |el, error| {
                el.child(div().p_4().text_color(cx.theme().danger).child(tr(error)))
            })
            .into_any_element()
    }
    fn database(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        if !state.state().is_unlocked() {
            return empty("ui.settings_locked", cx);
        }
        let Some(id) = state.state().database else {
            return empty("ui.settings_locked", cx);
        };
        let Some(database) = state.catalog().read(cx).database(id) else {
            return empty("ui.settings_locked", cx);
        };
        v_flex()
            .gap_3()
            .p_6()
            .max_w(rems(58.))
            .child(row("name", database.name.clone(), cx))
            .child(row("ui.description", database.description.clone(), cx))
            .child(row("ui.managing_device", "MacBook Pro", cx))
            .child(row("ui.storage", tr("ui.in_memory"), cx))
            .child(
                h_flex()
                    .px_6()
                    .gap_2()
                    .child(
                        Button::new("database-info")
                            .label(tr("ui.database_info"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                forms::database(this.store.clone(), Some(id), window, cx)
                            })),
                    )
                    .child(
                        Button::new("protection")
                            .label(tr("ui.protection"))
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let initial = this
                                    .protection
                                    .get(&id)
                                    .cloned()
                                    .unwrap_or_else(|| "1.0".into());
                                let settings = cx.entity().downgrade();
                                forms::text_form(
                                    "ui.protection",
                                    vec![("ui.kdf_seconds", initial, false)],
                                    Box::new(move |values, _, cx| {
                                        let value = values[0]
                                            .parse::<f64>()
                                            .map_err(|_| FormError::InvalidNumber)?;
                                        if !value.is_finite() || value <= 0. {
                                            return Err(FormError::InvalidNumber);
                                        }
                                        // Closing the settings view cancels this presentation-only change.
                                        let _ = settings.update(cx, |settings, cx| {
                                            settings.protection.insert(id, values[0].clone());
                                            cx.notify();
                                        });
                                        Ok(())
                                    }),
                                    window,
                                    cx,
                                )
                            })),
                    ),
            )
            .child(row(
                "ui.kdf_seconds",
                self.protection
                    .get(&id)
                    .cloned()
                    .unwrap_or_else(|| "1.0".into()),
                cx,
            ))
            .child(row("ui.attachment_limit", "100 MiB", cx))
            .child(
                div()
                    .px_6()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ui.settings_sample")),
            )
            .into_any_element()
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
                h_flex()
                    .h(rems(2.375))
                    .px_4()
                    .gap_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .children(
                        [
                            (SettingsTab::Device, "ui.device"),
                            (SettingsTab::Database, "ui.database"),
                        ]
                        .map(|(tab, label)| {
                            Button::new(label)
                                .ghost()
                                .rounded_none()
                                .h_full()
                                .selected(tab == self.tab)
                                .label(tr(label))
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.tab = tab;
                                    cx.notify();
                                }))
                        }),
                    ),
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
