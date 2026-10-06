//! Device preferences and authenticated shared database settings.

use crate::preferences::{FONT_SIZES, Language, ThemePreference};
use crate::{appearance::PreferencesStore, host::SettingsHost};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        Size,
        button::*,
        input::{InputEvent, InputState},
        menu::{DropdownMenu, PopupMenuItem},
        select::{Select, SelectEvent, SelectItem, SelectState},
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
#[derive(Clone)]
struct SettingChoice {
    key: &'static str,
    label: SharedString,
}
impl SelectItem for SettingChoice {
    type Value = &'static str;
    fn title(&self) -> SharedString {
        self.label.clone()
    }
    fn value(&self) -> &Self::Value {
        &self.key
    }
}
type SettingSelect = SelectState<Vec<SettingChoice>>;
fn row(label: &str, value: impl IntoElement, cx: &App) -> AnyElement {
    div()
        .px_6()
        .child(forms::form_row(label, None, value, cx))
        .into_any_element()
}
fn preference_button(id: &'static str, label: &str, value: SharedString, cx: &App) -> Button {
    Button::new(id)
        .ghost()
        .w_full()
        .h(rems(2.))
        .px(rems(0.625))
        .rounded(cx.theme().radius)
        .bg(cx.theme().title_bar)
        .text_color(cx.theme().foreground)
        .accessibility_label(tr(label))
        .dropdown_caret(true)
        .child(
            div()
                .text_size(rems(13. / 16.))
                .font_weight(FontWeight::NORMAL)
                .child(value),
        )
}
fn select_control(
    state: &Entity<SettingSelect>,
    id: &'static str,
    label: &str,
    cx: &App,
) -> Select<Vec<SettingChoice>> {
    Select::new(state)
        .id(id)
        .accessibility_label(tr(label))
        .appearance(false)
        .with_size(Size::Medium)
        .w_full()
        .h(rems(2.))
        .px(rems(0.625))
        .border_0()
        .rounded(rems(0.25))
        .bg(cx.theme().title_bar)
        .text_size(rems(13. / 16.))
        .font_weight(FontWeight::NORMAL)
        .text_color(cx.theme().foreground)
}
fn settings_section(title: &str, top: f32) -> AnyElement {
    div()
        .px_6()
        .pt(rems(top))
        .pb_4()
        .text_size(rems(1.))
        .line_height(rems(1.5))
        .font_weight(FontWeight::MEDIUM)
        .child(tr(title))
        .into_any_element()
}
fn choices(options: &[(&'static str, &'static str)]) -> Vec<SettingChoice> {
    options
        .iter()
        .map(|(key, label)| SettingChoice {
            key,
            label: tr(label),
        })
        .collect()
}
fn select_state(
    options: &[(&'static str, &'static str)],
    selected: &str,
    window: &mut Window,
    cx: &mut App,
) -> Entity<SettingSelect> {
    let index = options
        .iter()
        .position(|(key, _)| *key == selected)
        .map(IndexPath::new);
    cx.new(|cx| SelectState::new(choices(options), index, window, cx))
}
const IDLE_CHOICES: &[(&str, &str)] = &[
    ("60", "ui.minute"),
    ("300", "ui.five_minutes"),
    ("900", "ui.fifteen_minutes"),
    ("custom", "ui.custom"),
];
const CLIPBOARD_CHOICES: &[(&str, &str)] = &[
    ("never", "ui.never"),
    ("30", "ui.thirty_seconds"),
    ("60", "ui.minute"),
    ("custom", "ui.custom"),
];
const RELAY_CHOICES: &[(&str, &str)] = &[
    ("public", "sync.relay_public"),
    ("off", "sync.relay_off"),
    ("custom", "sync.relay_custom"),
];
fn idle_choice(seconds: u32) -> &'static str {
    match seconds {
        60 => "60",
        300 => "300",
        900 => "900",
        _ => "custom",
    }
}
fn clipboard_choice(seconds: Option<u32>) -> &'static str {
    match seconds {
        None => "never",
        Some(30) => "30",
        Some(60) => "60",
        _ => "custom",
    }
}
fn relay_choice(relay: &crate::local_settings::RelayPreference) -> &'static str {
    match relay {
        crate::local_settings::RelayPreference::Public => "public",
        crate::local_settings::RelayPreference::Disabled => "off",
        crate::local_settings::RelayPreference::Custom(_) => "custom",
    }
}
/// Device settings presentation; the database feature supplies its own content slot.
pub struct SettingsView<H: SettingsHost> {
    store: Entity<H>,
    preferences: Entity<PreferencesStore>,
    tab: SettingsTab,
    scroll: ScrollHandle,
    database: AnyView,
    device_name: Entity<InputState>,
    device_name_dirty: bool,
    device_name_pending: bool,
    device_name_error: Option<FormError>,
    idle: Entity<SettingSelect>,
    clipboard: Entity<SettingSelect>,
    relay: Entity<SettingSelect>,
    _subscriptions: Vec<Subscription>,
}
impl<H: SettingsHost> SettingsView<H> {
    fn refresh_controls(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.store.read(cx);
        let local = state.local(cx).clone();
        let idle = state.idle_seconds(cx);
        if !self.device_name_dirty && !self.device_name_pending {
            self.device_name.update(cx, |input, cx| {
                input.set_value(local.device_name.clone(), window, cx)
            });
        }
        let idle_key = idle_choice(idle);
        let clipboard_key = clipboard_choice(local.clipboard_seconds);
        let relay_key = relay_choice(&local.relay);
        for (state, items, key) in [
            (&self.idle, IDLE_CHOICES, idle_key),
            (&self.clipboard, CLIPBOARD_CHOICES, clipboard_key),
            (&self.relay, RELAY_CHOICES, relay_key),
        ] {
            state.update(cx, |state, cx| {
                state.set_items(choices(items), window, cx);
                state.set_selected_value(&key, window, cx);
            });
        }
    }
    fn commit_device_name(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.device_name_dirty
            || self.device_name_pending
            || self.store.read(cx).settings_busy(cx)
        {
            return;
        }
        let value = self.device_name.read(cx).value().to_string();
        if let Err(error) = taypeer_ui::require_name(&value) {
            self.device_name_error = Some(error);
            cx.notify();
            return;
        }
        let mut candidate = self.store.read(cx).local(cx).clone();
        if candidate.device_name == value {
            self.device_name_dirty = false;
            return;
        }
        candidate.device_name = value;
        self.device_name_pending = true;
        self.device_name_error = None;
        let view = cx.entity().downgrade();
        self.store.update(cx, |store, cx| {
            store.save_local_form(
                candidate,
                Box::new(move |result, _, cx| {
                    let _ = view.update(cx, |this, cx| {
                        this.device_name_pending = false;
                        this.device_name_error = result.err();
                        if result.is_ok() {
                            this.device_name_dirty = false;
                        }
                        cx.notify();
                    });
                }),
                window,
                cx,
            )
        });
        cx.notify();
    }
    fn custom_relay(&self, window: &mut Window, cx: &mut Context<Self>) {
        let value = match &self.store.read(cx).local(cx).relay {
            crate::local_settings::RelayPreference::Custom(url) => url.clone(),
            _ => String::new(),
        };
        let store = self.store.clone();
        forms::text_form(
            "sync.relay_custom",
            vec![("sync.relay_url", value, false)],
            Box::new(move |values, _, _| {
                let relay = crate::local_settings::RelayPreference::Custom(values[0].clone());
                relay.setting().map_err(FormError::Runtime)?;
                let store = store.clone();
                Ok(Some(Box::new(move |done, _, cx| {
                    store.update(cx, |s, cx| s.set_relay(relay, Some(done), cx))
                })))
            }),
            window,
            cx,
        );
    }
    /// Retain settings controls and subscriptions; the host owns requested changes.
    pub fn new(
        store: Entity<H>,
        preferences: Entity<PreferencesStore>,
        database: AnyView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = store.read(cx).local(cx).device_name.clone();
        let idle_key = idle_choice(store.read(cx).idle_seconds(cx));
        let clipboard_key = clipboard_choice(store.read(cx).local(cx).clipboard_seconds);
        let relay_key = relay_choice(&store.read(cx).local(cx).relay);
        let device_name = input(&name, false, window, cx);
        let idle = select_state(IDLE_CHOICES, idle_key, window, cx);
        let clipboard = select_state(CLIPBOARD_CHOICES, clipboard_key, window, cx);
        let relay = select_state(RELAY_CHOICES, relay_key, window, cx);
        let subscriptions = vec![
            cx.observe_in(&store, window, |this, _, window, cx| {
                this.refresh_controls(window, cx);
                cx.notify();
            }),
            cx.observe_in(&preferences, window, |this, _, window, cx| {
                this.refresh_controls(window, cx);
                cx.notify();
            }),
            cx.subscribe_in(
                &device_name,
                window,
                |this, input, event: &InputEvent, window, cx| match event {
                    InputEvent::Change => {
                        this.device_name_dirty = true;
                        // A required-name error stays beside an incomplete correction.
                        if taypeer_ui::require_name(input.read(cx).value().as_str()).is_ok() {
                            this.device_name_error = None;
                        }
                        cx.notify();
                    }
                    InputEvent::PressEnter {
                        secondary: false,
                        shift: false,
                    }
                    | InputEvent::Blur => this.commit_device_name(window, cx),
                    _ => {}
                },
            ),
            cx.subscribe_in(
                &idle,
                window,
                |this, _, event: &SelectEvent<Vec<SettingChoice>>, window, cx| match event {
                    SelectEvent::Confirm(Some("custom")) => {
                        this.refresh_controls(window, cx);
                        this.interval(false, window, cx);
                    }
                    SelectEvent::Confirm(Some(key)) => {
                        if let Ok(seconds) = key.parse() {
                            this.store
                                .update(cx, |store, cx| store.set_idle(seconds, cx));
                        }
                    }
                    SelectEvent::Confirm(None) => {}
                },
            ),
            cx.subscribe_in(
                &clipboard,
                window,
                |this, _, event: &SelectEvent<Vec<SettingChoice>>, window, cx| match event {
                    SelectEvent::Confirm(Some("custom")) => {
                        this.refresh_controls(window, cx);
                        this.interval(true, window, cx);
                    }
                    SelectEvent::Confirm(Some(key)) => {
                        let mut candidate = this.store.read(cx).local(cx).clone();
                        candidate.clipboard_seconds = if *key == "never" {
                            None
                        } else {
                            key.parse().ok()
                        };
                        this.store
                            .update(cx, |store, cx| store.save_local(candidate, cx));
                    }
                    SelectEvent::Confirm(None) => {}
                },
            ),
            cx.subscribe_in(
                &relay,
                window,
                |this, _, event: &SelectEvent<Vec<SettingChoice>>, window, cx| match event {
                    SelectEvent::Confirm(Some("custom")) => {
                        this.refresh_controls(window, cx);
                        this.custom_relay(window, cx);
                    }
                    SelectEvent::Confirm(Some(key)) => {
                        let choice = if *key == "off" {
                            crate::local_settings::RelayPreference::Disabled
                        } else {
                            crate::local_settings::RelayPreference::Public
                        };
                        this.store
                            .update(cx, |store, cx| store.set_relay(choice, None, cx));
                    }
                    SelectEvent::Confirm(None) => {}
                },
            ),
        ];
        Self {
            store,
            preferences,
            tab: SettingsTab::Device,
            database,
            scroll: ScrollHandle::new(),
            device_name,
            device_name_dirty: false,
            device_name_pending: false,
            device_name_error: None,
            idle,
            clipboard,
            relay,
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
            .py_5()
            .gap_1()
            .max_w(rems(53.))
            .child(row(
                "ui.device_name",
                v_flex()
                    .gap_1()
                    .child(
                        field(&self.device_name, "ui.device_name")
                            .id("device-name")
                            .disabled(
                                self.store.read(cx).settings_busy(cx) || self.device_name_pending,
                            )
                            .when(self.device_name_error.is_some(), |input| {
                                input.border_1().border_color(cx.theme().danger)
                            }),
                    )
                    .when_some(self.device_name_error, |el, error| {
                        el.child(
                            div()
                                .id("device-name-error")
                                .test_support()
                                .aria_label(tr(error.key()))
                                .text_xs()
                                .text_color(cx.theme().danger)
                                .child(tr(error.key())),
                        )
                    }),
                cx,
            ))
            .child(row(
                "language",
                preference_button("language", "language", language.label().into(), cx)
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
                preference_button("theme", "theme", tr(theme.key()), cx).dropdown_menu(
                    move |mut menu, _, _| {
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
                    },
                ),
                cx,
            ))
            .child(row(
                "font",
                preference_button("font-size", "font", size.to_string().into(), cx).dropdown_menu(
                    move |mut menu, _, _| {
                        for size in FONT_SIZES {
                            let target = sizes.clone();
                            menu = menu.item(PopupMenuItem::new(size.to_string()).on_click(
                                move |_, window, cx| {
                                    target.update(cx, |prefs, cx| prefs.set_font(size, window, cx))
                                },
                            ));
                        }
                        menu
                    },
                ),
                cx,
            ))
            .child(row(
                "ui.auto_lock",
                select_control(&self.idle, "autolock", "ui.auto_lock", cx)
                    .disabled(self.store.read(cx).settings_busy(cx)),
                cx,
            ))
            .child(row(
                "ui.clipboard_timeout",
                select_control(
                    &self.clipboard,
                    "clipboard-timeout",
                    "ui.clipboard_timeout",
                    cx,
                )
                .disabled(self.store.read(cx).settings_busy(cx)),
                cx,
            ))
            .when(cfg!(target_os = "macos"), |column| {
                column
                    .child(settings_section("ui.touch_id", 1.25))
                    .child(row(
                        "ui.biometric",
                        Button::new("biometric")
                            .disabled(true)
                            .label(tr("ui.touch_id_unavailable")),
                        cx,
                    ))
            })
            .child(settings_section("ui.connection", 0.75))
            .child(row(
                "ui.relay",
                select_control(&self.relay, "relay", "ui.relay", cx).disabled(
                    self.store.read(cx).settings_busy(cx) || self.store.read(cx).exchange_busy(cx),
                ),
                cx,
            ))
            .child(
                div()
                    .ml(rems(11.5))
                    .mt_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ui.relay_hint")),
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
            .font_weight(FontWeight::NORMAL)
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

#[cfg(feature = "component-rendering")]
/// Retain the real device-settings screen with public state and no runtime or file destination.
pub fn settings_preview(window: &mut Window, cx: &mut App) -> AnyView {
    struct PreviewHost(Entity<crate::local_settings::LocalSettings>);
    impl SettingsHost for PreviewHost {
        fn local<'a>(&self, cx: &'a App) -> &'a crate::local_settings::LocalSettings {
            self.0.read(cx)
        }
        fn idle_seconds(&self, _: &App) -> u32 {
            300
        }
        fn settings_busy(&self, _: &App) -> bool {
            false
        }
        fn exchange_busy(&self, _: &App) -> bool {
            false
        }
        fn set_idle(&mut self, _: u32, _: &mut Context<Self>) {}
        fn save_local(&mut self, _: crate::local_settings::LocalSettings, _: &mut Context<Self>) {}
        fn save_local_form(
            &mut self,
            _: crate::local_settings::LocalSettings,
            done: forms::Done,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            done(Err(FormError::Canceled), window, cx);
        }
        fn set_idle_form(
            &mut self,
            _: u32,
            done: forms::Done,
            window: &mut Window,
            cx: &mut Context<Self>,
        ) {
            done(Err(FormError::Canceled), window, cx);
        }
        fn set_relay(
            &mut self,
            _: crate::local_settings::RelayPreference,
            _: Option<forms::Done>,
            _: &mut Context<Self>,
        ) {
        }
        fn settings(&mut self, _: &mut Context<Self>) {}
    }
    struct DatabaseSlot;
    impl Render for DatabaseSlot {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div()
        }
    }
    let local = crate::local_settings::LocalSettings {
        device_name: "MacBook Pro".into(),
        ..Default::default()
    };
    let local = cx.new(|_| local);
    let host = cx.new(|_| PreviewHost(local));
    let values = crate::preferences::Preferences {
        language: if rust_i18n::locale().starts_with("ru") {
            Language::Russian
        } else {
            Language::English
        },
        theme: if cx.theme().mode.is_dark() {
            ThemePreference::Dark
        } else {
            ThemePreference::Light
        },
        font_size: f32::from(window.rem_size()).round() as u8,
        ..Default::default()
    };
    let preferences = cx.new(|cx| PreferencesStore::preview(values, cx));
    let database = cx.new(|_| DatabaseSlot).into();
    cx.new(|cx| SettingsView::new(host, preferences, database, window, cx))
        .into()
}
