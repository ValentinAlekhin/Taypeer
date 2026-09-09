//! GPUI window coordinator. Owns session-bound state; child modules implement
//! cohesive transitions and views without exposing them outside the client.

mod details;
mod dialogs;
mod editor;
mod entries;
mod files;
mod groups;
mod inspector;
mod navigation;
mod queries;
mod settings;
mod shell;

use crate::macos::actions::{CancelEditing, FocusSearch, LockDatabase, SaveEntry};
use crate::macos::common::{input, tr};
use crate::preferences::Preferences;
use editor::Editor;
use gpui_kit::component::input::{InputEvent, InputState};
use gpui_kit::component::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;
use std::collections::{BTreeMap, BTreeSet};
use taypeer_services::{DatabaseId, DatabaseService, EntryId, GroupId, RevisionId, SessionToken};

#[derive(Clone, Copy, Default, PartialEq, Eq)]
enum EntryTab {
    #[default]
    Overview,
    Attributes,
    History,
}
impl EntryTab {
    const ALL: [Self; 3] = [Self::Overview, Self::Attributes, Self::History];
    fn key(self) -> &'static str {
        match self {
            Self::Overview => "overview",
            Self::Attributes => "attributes",
            Self::History => "history",
        }
    }
}

#[derive(Clone)]
enum Navigation {
    Form(Form, String),
    Database(DatabaseId),
    Group(GroupId),
    Entry(EntryId),
    NewEntry,
    OpenFile(std::path::PathBuf),
    Cancel,
}
#[derive(Clone)]
enum Form {
    Database,
    OpenFile(std::path::PathBuf),
    Group(Option<GroupId>),
    Rename(GroupId),
}
// Distinct owners make rebinding one input independent of other listeners.
struct WindowSubscriptions {
    _search: Subscription,
    _appearance: Subscription,
    _activation: Subscription,
    prompts: Vec<Subscription>,
}

pub(super) struct Client {
    service: DatabaseService,
    database: Option<DatabaseId>,
    session: Option<SessionToken>,
    sessions: BTreeMap<DatabaseId, SessionToken>,
    group: Option<GroupId>,
    selected: Option<EntryId>,
    editor: Option<Editor>,
    password: Entity<InputState>,
    search: Entity<InputState>,
    form_input: Entity<InputState>,
    file_password: Entity<InputState>,
    file_confirmation: Entity<InputState>,
    busy: bool,
    demo_mode: bool,
    lock_requested: bool,
    io_task: Option<Task<()>>,
    form: Option<Form>,
    settings: bool,
    root_focus: FocusHandle,
    modal_focus: FocusHandle,
    group_focus: FocusHandle,
    entry_focus: FocusHandle,
    group_scroll: ScrollHandle,
    entry_scroll: ScrollHandle,
    pending: Option<Navigation>,
    restore: bool,
    tab: EntryTab,
    revision: Option<RevisionId>,
    revealed: BTreeMap<String, SharedString>,
    collapsed: BTreeSet<GroupId>,
    descending: bool,
    prefs: Preferences,
    invalid_prefs: bool,
    error: Option<&'static str>,
    subscriptions: WindowSubscriptions,
}

impl Client {
    pub(super) fn new(window: &mut Window, cx: &mut Context<Self>, demo_mode: bool) -> Self {
        let (prefs, invalid_prefs) = Preferences::load();
        rust_i18n::set_locale(prefs.language.code());
        let password = input("", true, window, cx);
        let search = input("", false, window, cx);
        let form_input = input("", false, window, cx);
        let subscriptions = WindowSubscriptions {
            _search: cx.subscribe_in(&search, window, |_, _, _: &InputEvent, _, cx| cx.notify()),
            _appearance: cx
                .observe_window_appearance(window, |this, window, cx| this.apply_theme(window, cx)),
            _activation: cx
                .observe_window_activation(window, |this, window, cx| this.apply_theme(window, cx)),
            prompts: Vec::new(),
        };
        let mut this = Self {
            service: DatabaseService::new(),
            database: None,
            session: None,
            sessions: BTreeMap::new(),
            group: None,
            selected: None,
            editor: None,
            password,
            search,
            form_input,
            file_password: input("", true, window, cx),
            file_confirmation: input("", true, window, cx),
            busy: false,
            demo_mode,
            lock_requested: false,
            io_task: None,
            form: None,
            settings: false,
            root_focus: cx.focus_handle(),
            modal_focus: cx.focus_handle(),
            group_focus: cx.focus_handle(),
            entry_focus: cx.focus_handle(),
            group_scroll: ScrollHandle::new(),
            entry_scroll: ScrollHandle::new(),
            pending: None,
            restore: false,
            tab: EntryTab::Overview,
            revision: None,
            revealed: BTreeMap::new(),
            collapsed: BTreeSet::new(),
            descending: false,
            prefs,
            invalid_prefs,
            error: invalid_prefs.then_some("prefs_error"),
            subscriptions,
        };
        this.bind_prompt_inputs(window, cx);
        this.apply_theme(window, cx);
        this
    }

    pub(super) fn bind_prompt_inputs(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.subscriptions.prompts.clear();
        self.subscriptions.prompts.push(cx.subscribe_in(
            &self.password,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.password.entity_id() == state.entity_id()
                    && this.session.is_none()
                    && !this.settings
                    && this.form.is_none()
                {
                    this.unlock(window, cx);
                }
            },
        ));
        self.subscriptions.prompts.push(cx.subscribe_in(
            &self.form_input,
            window,
            |this, state, event: &InputEvent, window, cx| {
                if matches!(event, InputEvent::PressEnter { .. })
                    && this.form_input.entity_id() == state.entity_id()
                    && this.form.is_some()
                {
                    this.commit_form(window, cx);
                }
            },
        ));
        for field in [&self.file_password, &self.file_confirmation] {
            self.subscriptions.prompts.push(cx.subscribe_in(
                field,
                window,
                |this, state, event: &InputEvent, window, cx| {
                    let current = state.entity_id() == this.file_password.entity_id()
                        || state.entity_id() == this.file_confirmation.entity_id();
                    if current
                        && !this.busy
                        && this.form.is_some()
                        && matches!(event, InputEvent::PressEnter { .. })
                    {
                        this.commit_form(window, cx);
                    }
                },
            ));
        }
        self.update_placeholders(window, cx);
    }

    pub(super) fn update_placeholders(&self, window: &mut Window, cx: &mut App) {
        self.search.update(cx, |state, cx| {
            state.set_placeholder(tr("search"), window, cx)
        });
        self.password.update(cx, |state, cx| {
            state.set_placeholder(tr("password"), window, cx)
        });
        self.form_input.update(cx, |state, cx| {
            state.set_placeholder(tr("name"), window, cx)
        });
    }
}
impl Render for Client {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.busy {
            return v_flex()
                .size_full()
                .items_center()
                .justify_center()
                .gap_4()
                .track_focus(&self.root_focus)
                .key_context("Taypeer")
                .on_action(cx.listener(|this, _: &LockDatabase, window, cx| this.lock(window, cx)))
                .child(tr("file_working"))
                .child(
                    gpui_kit::component::button::Button::new("busy-lock")
                        .label(tr("lock"))
                        .on_click(cx.listener(|this, _, window, cx| this.lock(window, cx))),
                )
                .into_any_element();
        }
        v_flex()
            .track_focus(&self.root_focus)
            .key_context("Taypeer")
            .on_action(cx.listener(|this, _: &FocusSearch, window, cx| {
                if this.session.is_some()
                    && this.form.is_none()
                    && this.pending.is_none()
                    && !this.settings
                    && !this.restore
                {
                    this.search.update(cx, |state, cx| state.focus(window, cx));
                }
            }))
            .on_action(cx.listener(|this, _: &SaveEntry, window, cx| {
                if this.form.is_none() && !this.settings && this.pending.is_none() {
                    this.save(window, cx);
                }
            }))
            .on_action(cx.listener(|this, _: &LockDatabase, window, cx| this.lock(window, cx)))
            .on_action(cx.listener(|this, _: &CancelEditing, window, cx| {
                if this.settings {
                    this.settings = false;
                    this.root_focus.focus(window, cx);
                } else if this.form.is_some() {
                    this.cancel_form(window, cx);
                } else if this.pending.is_some() {
                    this.pending = None;
                    this.root_focus.focus(window, cx);
                } else if this.editor.is_some() {
                    this.navigate(Navigation::Cancel, window, cx);
                }
                cx.notify();
            }))
            .relative()
            .size_full()
            .bg(cx.theme().background)
            .text_color(cx.theme().foreground)
            .child(self.header(cx))
            .child(
                div()
                    .px_3()
                    .py_1()
                    .text_xs()
                    .text_color(cx.theme().warning)
                    .child(tr(if self.demo_mode {
                        "demo"
                    } else {
                        "file_experimental"
                    })),
            )
            .child(div().flex_1().min_h_0().child(if self.session.is_some() {
                self.workspace(cx)
            } else {
                self.unlock_panel(cx)
            }))
            .when_some(self.error, |el, error| {
                el.child(
                    div()
                        .px_3()
                        .py_1()
                        .text_color(cx.theme().danger)
                        .child(tr(error)),
                )
            })
            .child(
                div()
                    .px_3()
                    .py_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(
                        if self
                            .database
                            .as_ref()
                            .is_some_and(|id| self.service.is_file(id))
                        {
                            "file_autosave"
                        } else {
                            "file_welcome"
                        },
                    )),
            )
            .children(self.overlay(cx))
            .into_any_element()
    }
}
