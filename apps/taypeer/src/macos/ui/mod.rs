//! GPUI Kit screens backed by independent presentation stores and the Rust runtime.

use gpui_kit::prelude::FluentBuilder;
mod appearance;
mod clipboard;
mod editor;
mod entries;
mod forms;
mod generator;
mod header;
mod icons;
mod images;
mod inspector;
mod read_value;
mod session;
mod settings;
mod sidebar;
mod style;
mod workspace;

use crate::{
    macos::actions::{CancelEditing, FocusSearch, LockDatabase, SaveEntry},
    ui_state::*,
};
use appearance::PreferencesStore;
use gpui_kit::{component::*, *};
use style::tr;
use workspace::WorkspaceStore;

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
        KeyBinding::new("cmd-n", NewEntry, Some("Taypeer")),
        KeyBinding::new("cmd-e", EditEntry, Some("Taypeer")),
        KeyBinding::new("cmd-o", OpenDatabase, Some("Taypeer")),
        KeyBinding::new("cmd-shift-n", CreateDatabase, Some("Taypeer")),
        KeyBinding::new("cmd-,", ShowSettings, Some("Taypeer")),
        KeyBinding::new("cmd-w", CloseWindow, Some("Taypeer")),
        KeyBinding::new("cmd-q", Quit, None),
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
    header: Entity<header::Header>,
    sidebar: Entity<sidebar::Sidebar>,
    entries: Entity<entries::Entries>,
    inspector: Option<Entity<inspector::Inspector>>,
    session: Option<Entity<session::SessionView>>,
    settings: Entity<settings::SettingsView>,
    focus: FocusHandle,
    layout: Option<((Pixels, u8, bool), Entity<ResizableState>)>,
    editing: bool,
    unlocked: bool,
    _subscriptions: Vec<Subscription>,
}

impl AppView {
    pub fn new(window: &mut Window, cx: &mut Context<Self>) -> Self {
        let preferences = cx.new(|_| PreferencesStore::load());
        rust_i18n::set_locale(preferences.read(cx).values().language.code());
        preferences.update(cx, |prefs, cx| prefs.apply(window, cx));
        cx.set_global(images::Images::default());
        let catalog = cx.new(|_| CatalogStore::default());
        let store = cx.new(|cx| WorkspaceStore::new(catalog, cx));
        store.update(cx, |_, cx| {
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
            .detach();
        });
        let header = cx.new(|cx| header::Header::new(store.clone(), cx));
        let sidebar = cx.new(|cx| sidebar::Sidebar::new(store.clone(), window, cx));
        let entries = cx.new(|cx| entries::Entries::new(store.clone(), window, cx));
        let settings = cx
            .new(|cx| settings::SettingsView::new(store.clone(), preferences.clone(), window, cx));
        let subscriptions = vec![
            cx.observe_in(&store, window, |this, store, window, cx| {
                let editing = store.read(cx).editor().is_some();
                let unlocked = store.read(cx).state().is_unlocked();
                if (this.editing && !editing) || (!this.unlocked && unlocked) {
                    this.focus.focus(window, cx);
                }
                this.editing = editing;
                this.unlocked = unlocked;
                cx.notify();
            }),
            cx.observe_keystrokes(|this, _, _, cx| this.store.read(cx).activity()),
            cx.observe(&preferences, |_, _, cx| cx.notify()),
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
                        forms::request_quit(store, window, cx);
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
        Self {
            store,
            preferences,
            header,
            sidebar,
            entries,
            inspector: None,
            session: None,
            settings,
            focus,
            layout: None,
            editing: false,
            unlocked: false,
            _subscriptions: subscriptions,
        }
    }

    fn body(&mut self, window: &mut Window, cx: &mut Context<Self>) -> AnyElement {
        let state = &self.store.read(cx).state();
        let unlocked = state.is_unlocked();
        let route = state.route;
        let show_inspector =
            unlocked && (state.selected.is_some() || self.store.read(cx).editor().is_some());
        if !unlocked {
            self.inspector = None;
        }
        if route == Route::Settings {
            return self.settings.clone().into_any_element();
        }
        if route == Route::Welcome || !unlocked {
            if self.session.is_none() {
                self.session =
                    Some(cx.new(|cx| session::SessionView::new(self.store.clone(), window, cx)));
            }
            return self
                .session
                .as_ref()
                .expect("created session view")
                .clone()
                .into_any_element();
        }
        self.session = None;
        if show_inspector && self.inspector.is_none() {
            self.inspector =
                Some(cx.new(|cx| inspector::Inspector::new(self.store.clone(), window, cx)));
        }
        if !show_inspector {
            self.inspector = None;
        }
        let prefs = self.preferences.read(cx).values().clone();
        let scale = prefs.font_size as f32 / 16.;
        let preferences = self.preferences.clone();
        let layout_key = (
            window.viewport_size().width,
            prefs.font_size,
            show_inspector,
        );
        if self
            .layout
            .as_ref()
            .is_none_or(|(key, _)| *key != layout_key)
        {
            self.layout = Some((layout_key, cx.new(|_| ResizableState::default())));
        }
        let layout = self.layout.as_ref().expect("layout initialized").1.clone();
        let inspector_width = (prefs.inspector_width * scale).min(
            (f32::from(window.viewport_size().width) - prefs.group_width * scale - 330.).max(380.),
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
                .size_range(px(180.)..px(280. * scale))
                .child(self.sidebar.clone()),
        )
        .child(
            resizable_panel()
                .size_range(px(330.)..Pixels::MAX)
                .child(self.entries.clone()),
        );
        if let Some(inspector) = self.inspector.as_ref() {
            panes = panes.child(
                resizable_panel()
                    .size(px(inspector_width))
                    .flex_none()
                    .size_range(px(380.)..Pixels::MAX)
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
        let suspended = self
            .store
            .read(cx)
            .state()
            .database
            .as_ref()
            .is_some_and(|db| self.store.read(cx).state().suspended.contains_key(db));
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
        let active = store.state().is_unlocked();
        let file_bytes = store
            .state()
            .database
            .as_ref()
            .and_then(|db| store.catalog().read(cx).database(db))
            .map_or(0, |db| db.file_bytes);
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
                if this.store.read(cx).state().is_unlocked() {
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
                    if store.state().route == Route::Settings {
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
                forms::choose_file(this.store.clone(), window, cx)
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
                    .when(active, |el| {
                        el.child(tr("ui.no_sync")).child(style::icon("refresh-cw"))
                    }),
            )
            .children(Root::render_dialog_layer(window, cx))
            .children(Root::render_notification_layer(window, cx))
    }
}
