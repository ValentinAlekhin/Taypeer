//! GPUI Kit screens backed by independent presentation stores and the Rust runtime.

use gpui_kit::prelude::FluentBuilder;
use taypeer_settings_ui::appearance;

use taypeer_settings_ui as settings;

use taypeer_sync_ui as sync;
use taypeer_ui::style;

use crate::desktop::actions::{CancelEditing, FocusSearch, LockDatabase, SaveEntry};
use appearance::PreferencesStore;
use gpui_kit::{
    component::{button::ButtonVariants, *},
    *,
};
use style::tr;
use taypeer_database_ui as database;
use taypeer_database_ui::{
    DatabaseSettings, Destination, Entries, Header, Inspector, Route, SessionView, Sidebar,
    WorkspaceStore,
};

actions!(
    ui,
    [
        NewEntry,
        EditEntry,
        OpenDatabase,
        CreateDatabase,
        ShowSettings,
        CloseWindow,
        Quit
    ]
);
pub(super) fn bind(cx: &mut App) {
    cx.bind_keys([
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("n"),
            NewEntry,
            Some("Taypeer"),
        ),
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("e"),
            EditEntry,
            Some("Taypeer"),
        ),
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("o"),
            OpenDatabase,
            Some("Taypeer"),
        ),
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("shift-n"),
            CreateDatabase,
            Some("Taypeer"),
        ),
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut(","),
            ShowSettings,
            Some("Taypeer"),
        ),
        KeyBinding::new(
            &taypeer_desktop_platform::primary_shortcut("w"),
            CloseWindow,
            Some("Taypeer"),
        ),
        KeyBinding::new(&taypeer_desktop_platform::primary_shortcut("q"), Quit, None),
    ]);
}

pub(super) fn app_menu(cx: &mut App) {
    cx.set_menus(vec![Menu {
        name: "Taypeer".into(),
        items: vec![MenuItem::action(tr("ui.quit"), Quit)],
        disabled: false,
    }]);
}

pub(super) struct AppView {
    store: Entity<WorkspaceStore>,
    preferences: Entity<PreferencesStore>,
    header: Entity<Header>,
    sidebar: Entity<Sidebar>,
    entries: Entity<Entries>,
    inspector: Option<Entity<Inspector>>,
    session: Option<Entity<SessionView>>,
    settings: Entity<settings::SettingsView<WorkspaceStore>>,
    sync: Entity<sync::SyncView<WorkspaceStore>>,
    focus: FocusHandle,
    layout: Option<((Pixels, u8, bool), Entity<ResizableState>)>,
    editing: bool,
    unlocked: bool,
    _subscriptions: Vec<Subscription>,
    _poll: Task<()>,
}

impl AppView {
    #[cfg(feature = "ui-test-support")]
    pub(super) fn test_capture_allowed(&self, cx: &App) -> bool {
        !self.editing
            && self
                .session
                .as_ref()
                .is_none_or(|session| session.read(cx).test_capture_allowed(cx))
            && !self.store.read(cx).sync(cx).has_invitation()
            && !self
                .inspector
                .as_ref()
                .is_some_and(|inspector| inspector.read(cx).has_revealed_values())
    }

    #[cfg(feature = "ui-test-support")]
    pub(super) fn test_idle(&self, cx: &App) -> bool {
        self.store.read(cx).test_idle(cx)
    }
    #[cfg(feature = "ui-test-support")]
    pub(super) fn test_controls(&self, cx: &App) -> Vec<taypeer_runtime::WorkerControl> {
        self.store.read(cx).test_controls()
    }
    #[cfg(feature = "ui-test-support")]
    pub(super) fn test_status(&self, cx: &App) -> String {
        self.store.read(cx).test_status(cx)
            + &self
                .session
                .as_ref()
                .map(|view| format!("; {}", view.read(cx).test_status(cx)))
                .unwrap_or_default()
    }

    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        #[cfg(feature = "ui-test-support")]
        let path = cx
            .try_global::<super::TestLaunch>()
            .map(|config| config.preferences.clone());
        #[cfg(not(feature = "ui-test-support"))]
        let path = None;
        let path = path.or_else(|| {
            cx.global::<taypeer_ui::LaunchProfile>()
                .0
                .clone()
                .or_else(|| taypeer_desktop_platform::profile_path().ok())
                .map(|p| p.join("preferences.toml"))
        });
        let preferences = cx.new(|cx| PreferencesStore::load(path, cx));
        rust_i18n::set_locale(preferences.read(cx).values().language.code());
        preferences.update(cx, |prefs, cx| prefs.apply(window, cx));
        app_menu(cx);
        database::init(cx);
        let profile = cx
            .global::<taypeer_ui::LaunchProfile>()
            .0
            .clone()
            .map(Ok)
            .unwrap_or_else(taypeer_desktop_platform::profile_path)
            .map_err(|_| taypeer_runtime::RuntimeError::Transport);
        #[cfg(feature = "ui-test-support")]
        let backend = if let Some(config) = cx.try_global::<taypeer_ui::TestLaunch>() {
            profile.and_then(|profile| {
                crate::backend::Backend::configured(profile, config.worker.clone(), true)
            })
        } else {
            profile.and_then(crate::backend::Backend::new)
        };
        #[cfg(not(feature = "ui-test-support"))]
        let backend = profile.and_then(crate::backend::Backend::new);
        let notice = if let Some(platform) = cx.try_global::<taypeer_desktop_platform::Platform>() {
            if let Ok(backend) = &backend {
                platform.attach(backend.sessions.clone());
            }
            backend.as_ref().err().map(crate::backend::error_key)
        } else {
            Some("ui.platform_unavailable")
        };
        let backend = backend.ok().map(std::sync::Arc::new);
        let device_settings = cx.new(|cx| {
            taypeer_settings_ui::state::DeviceSettingsState::new(backend.clone(), window, cx)
        });
        let exchange = cx.new(|cx| {
            taypeer_sync_ui::state::SyncState::new(backend.clone(), device_settings.clone(), cx)
        });
        let store = cx.new(|cx| {
            WorkspaceStore::new(
                backend,
                device_settings.clone(),
                exchange,
                notice,
                window,
                cx,
            )
        });
        let poll = store.update(cx, |_, cx| {
            cx.spawn_in(window, async move |store, cx| {
                loop {
                    cx.background_executor()
                        .timer(std::time::Duration::from_millis(50))
                        .await;
                    if store
                        .update_in(cx, |store, window, cx| store.poll(window, cx))
                        .is_err()
                    {
                        break;
                    }
                }
            })
        });
        let header = cx.new(|cx| Header::new(store.clone(), cx));
        let sidebar = cx.new(|cx| Sidebar::new(store.clone(), window, cx));
        let entries = cx.new(|cx| Entries::new(store.clone(), window, cx));
        let database_settings = cx.new(|cx| DatabaseSettings::new(store.clone(), cx));
        let settings = cx.new(|cx| {
            settings::SettingsView::new(
                store.clone(),
                preferences.clone(),
                database_settings.into(),
                window,
                cx,
            )
        });
        let sync = cx.new(|cx| sync::SyncView::new(store.clone(), window, cx));
        let subscriptions = vec![
            cx.observe(&device_settings, |_, settings, cx| {
                let seconds = settings.read(cx).values().clipboard_seconds;
                if cx.has_global::<taypeer_desktop_platform::Platform>() {
                    cx.global_mut::<taypeer_desktop_platform::Platform>()
                        .set_clipboard_seconds(seconds);
                }
                cx.notify();
            }),
            cx.observe_in(&store, window, |this, store, window, cx| {
                let editing = store.read(cx).is_editing();
                let unlocked = store.read(cx).is_unlocked();
                if (this.editing && !editing) || (!this.unlocked && unlocked) {
                    this.focus.focus(window, cx);
                }
                this.editing = editing;
                this.unlocked = unlocked;
                this.reconcile_views(window, cx);
                cx.notify();
            }),
            cx.observe_keystrokes(|this, _, _, cx| this.store.read(cx).activity()),
            cx.observe_in(&preferences, window, |this, _, window, cx| {
                app_menu(cx);
                this.reconcile_layout(window, cx);
                cx.notify();
            }),
            cx.observe_window_bounds(window, |this, window, cx| {
                this.reconcile_layout(window, cx);
                cx.notify();
            }),
            cx.observe_window_appearance(window, |this, window, cx| {
                this.preferences
                    .update(cx, |prefs, cx| prefs.apply(window, cx))
            }),
            cx.observe_window_activation(window, |this, window, cx| {
                this.preferences
                    .update(cx, |prefs, cx| prefs.apply(window, cx))
            }),
        ];
        let quit_store = store.downgrade();
        let quit_window = window.window_handle();
        App::on_action(cx, move |_: &Quit, cx| {
            let store = quit_store.clone();
            // Global actions run while the dispatching window is borrowed by GPUI.
            // Re-enter it only after dispatch has returned the window to the app.
            cx.defer(move |cx| {
                let result = quit_window.update(cx, |_, window, cx| {
                    if let Some(store) = store.upgrade() {
                        database::request_quit(store, window, cx);
                    } else {
                        cx.quit();
                    }
                });
                if result.is_err() && cx.windows().is_empty() {
                    cx.quit();
                }
            });
        });
        let weak = store.downgrade();
        window.on_window_should_close(cx, move |window, cx| {
            if window.has_active_dialog(cx) {
                return false;
            }
            weak.update(cx, |this, cx| {
                if this.dirty(cx) {
                    this.navigate(Destination::Quit, window, cx);
                    false
                } else {
                    true
                }
            })
            .unwrap_or(true)
        });
        let focus = cx.focus_handle();
        focus.focus(window, cx);
        let mut view = Self {
            store,
            preferences,
            header,
            sidebar,
            entries,
            inspector: None,
            session: None,
            settings,
            sync,
            focus,
            layout: None,
            editing: false,
            unlocked: false,
            _subscriptions: subscriptions,
            _poll: poll,
        };
        view.reconcile_views(window, cx);
        view
    }

    fn reconcile_views(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let state = self.store.read(cx);
        let route = state.route();
        let session = (route == Route::Welcome || !state.is_unlocked())
            && !matches!(route, Route::Settings | Route::Devices | Route::Receive);
        let inspector = state.has_inspector();
        if session && self.session.is_none() {
            self.session = Some(cx.new(|cx| SessionView::new(self.store.clone(), window, cx)));
        } else if !session {
            self.session = None;
        }
        if inspector && self.inspector.is_none() {
            self.inspector = Some(cx.new(|cx| Inspector::new(self.store.clone(), window, cx)));
        } else if !inspector {
            self.inspector = None;
        }
        self.reconcile_layout(window, cx);
    }

    fn reconcile_layout(&mut self, window: &Window, cx: &mut Context<Self>) {
        let key = (
            window.viewport_size().width,
            self.preferences.read(cx).values().font_size,
            self.store.read(cx).has_inspector(),
        );
        if self
            .layout
            .as_ref()
            .is_none_or(|(previous, _)| *previous != key)
        {
            self.layout = Some((key, cx.new(|_| ResizableState::default())));
        }
    }

    fn body(&self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let unlocked = state.is_unlocked();
        let route = state.route();
        let show_inspector = state.has_inspector();
        if route == Route::Receive || route == Route::Devices {
            return h_flex()
                .items_stretch()
                .size_full()
                .when(route == Route::Devices && unlocked, |el| {
                    el.child(
                        div()
                            .w(rems(14.))
                            .h_full()
                            .flex_shrink_0()
                            .child(self.sidebar.clone()),
                    )
                })
                .child(div().flex_1().min_w_0().child(self.sync.clone()))
                .into_any_element();
        }
        if route == Route::Settings {
            return self.settings.clone().into_any_element();
        }
        if route == Route::Welcome || !unlocked {
            return self
                .session
                .as_ref()
                .expect("created session view")
                .clone()
                .into_any_element();
        }
        let prefs = self.preferences.read(cx).values().clone();
        let scale = prefs.font_size as f32 / 16.;
        let preferences = self.preferences.clone();
        let layout = self.layout.as_ref().expect("layout initialized").1.clone();
        let inspector_width = (prefs.inspector_width * scale).min(
            (f32::from(window.viewport_size().width) - prefs.group_width * scale - 330. * scale)
                .max(380. * scale),
        );
        let mut panes = h_resizable(if show_inspector {
            "workspace-three"
        } else {
            "workspace-two"
        })
        .with_state(&layout)
        .child(
            resizable_panel()
                .size(px(prefs.group_width * scale))
                .flex_none()
                .size_range(px(192. * scale)..px(280. * scale))
                .child(self.sidebar.clone()),
        )
        .child(
            resizable_panel()
                .size_range(px(330. * scale)..Pixels::MAX)
                .child(self.entries.clone()),
        );
        if let Some(inspector) = self.inspector.as_ref() {
            panes = panes.child(
                resizable_panel()
                    .size(px(inspector_width))
                    .flex_none()
                    .size_range(px(380. * scale)..Pixels::MAX)
                    .child(inspector.clone()),
            );
        }
        let panes = panes.on_resize(move |state: &Entity<ResizableState>, _, cx| {
            let sizes = state.read(cx).sizes();
            if let Some(group) = sizes.first() {
                let group = f32::from(*group) / scale;
                let entry = show_inspector
                    .then(|| sizes.get(2).map(|v| f32::from(*v) / scale))
                    .flatten();
                preferences.update(cx, |prefs, cx| prefs.resize(group, entry, cx));
            }
        });
        let suspended = self.store.read(cx).has_suspended_draft();
        v_flex()
            .size_full()
            .when(suspended, |el| {
                el.child(
                    h_flex()
                        .p_3()
                        .gap_3()
                        .child(tr("ui.draft_available"))
                        .child(
                            gpui_kit::component::button::Button::new("restore-draft")
                                .label(tr("ui.restore_draft"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store
                                        .update(cx, |store, cx| store.restore_draft(true, cx))
                                })),
                        )
                        .child(
                            gpui_kit::component::button::Button::new("discard-draft")
                                .label(tr("ui.discard_draft"))
                                .on_click(cx.listener(|this, _, _, cx| {
                                    this.store
                                        .update(cx, |store, cx| store.restore_draft(false, cx))
                                })),
                        ),
                )
            })
            .child(div().flex_1().min_h_0().child(panes))
            .into_any_element()
    }
}

impl Render for AppView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let body = self.body(window, cx);
        let store = self.store.read(cx);
        let active = store.is_unlocked();
        let file_bytes = store.file_bytes(cx);
        let notice = store.notice().or(self.preferences.read(cx).error());
        v_flex()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .text_sm()
            .track_focus(&self.focus)
            .key_context("Taypeer")
            .capture_key_down(cx.listener(|this, _, _, cx| this.store.read(cx).activity()))
            .capture_any_mouse_down(cx.listener(|this, _, _, cx| this.store.read(cx).activity()))
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                if this.store.read(cx).is_unlocked() {
                    this.entries
                        .update(cx, |entry, cx| entry.focus_search(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &SaveEntry, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                this.store.update(cx, |store, cx| {
                    store.save(cx);
                });
            }))
            .on_action(cx.listener(|this, _: &EditEntry, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                this.store.update(cx, |store, cx| store.begin_edit(cx))
            }))
            .on_action(cx.listener(|this, _: &NewEntry, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                this.store.update(cx, |store, cx| {
                    store.navigate(Destination::NewEntry, window, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &LockDatabase, window, cx| {
                this.store.update(cx, |store, cx| store.lock(window, cx))
            }))
            .on_action(cx.listener(|this, _: &CancelEditing, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                this.store.update(cx, |store, cx| {
                    if store.route() == Route::Settings {
                        store.settings(cx);
                    } else {
                        store.navigate(Destination::CancelEdit, window, cx);
                    }
                });
            }))
            .on_action(cx.listener(|this, _: &ShowSettings, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                this.store.update(cx, |store, cx| store.settings(cx))
            }))
            .on_action(cx.listener(|this, _: &OpenDatabase, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                database::choose_file(this.store.clone(), window, cx)
            }))
            .on_action(cx.listener(|this, _: &CreateDatabase, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                this.store.update(cx, |store, cx| {
                    store.navigate(Destination::CreateDatabase, window, cx)
                })
            }))
            .on_action(cx.listener(|this, _: &CloseWindow, window, cx| {
                if window.has_active_dialog(cx) {
                    return;
                }
                this.store.update(cx, |store, cx| {
                    store.navigate(Destination::Quit, window, cx)
                })
            }))
            .child(self.header.clone())
            .child(div().flex_1().min_h_0().child(body))
            .child(
                h_flex()
                    .h(rems(1.75))
                    .px_3()
                    .gap_2()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .when(active, |el| {
                        el.child(style::icon("database"))
                            .child(style::file_size(file_bytes))
                    })
                    .when_some(notice, |el, notice| el.child(tr(notice)))
                    .child(div().flex_1())
                    .when(store.selected_database().is_some(), |el| {
                        el.child(
                            button::Button::new("sync-status")
                                .ghost()
                                .compact()
                                .label(tr(store
                                    .sync(cx)
                                    .database_status(store.selected_database())))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.store.update(cx, |s, cx| {
                                        s.navigate(Destination::Devices, window, cx)
                                    })
                                })),
                        )
                    }),
            )
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
