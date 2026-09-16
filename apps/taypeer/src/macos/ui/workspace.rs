//! Window navigation and acceptance of generation-scoped background results.
mod sync;
use super::style::tr;
use crate::{
    backend::{Backend, Connection, Query, Ticket},
    ui_state::*,
};
use gpui_kit::{
    component::{button::*, *},
    *,
};
use std::{collections::BTreeMap, path::PathBuf};
use taypeer_runtime::{Command, RuntimeError, session::LockReason};
type Pending =
    Box<dyn FnMut(&mut WorkspaceStore, &mut Window, &mut Context<WorkspaceStore>) -> bool>;
pub(super) struct WorkspaceStore {
    sync: SyncStore,
    state: NavigationState,
    catalog: Entity<CatalogStore>,
    editor: Option<Entity<EditorStore>>,
    backend: Option<Backend>,
    connections: BTreeMap<DatabaseId, Connection>,
    requests: BTreeMap<DatabaseId, u64>,
    reported_locks: std::collections::BTreeSet<(DatabaseId, u64)>,
    pending: Vec<Pending>,
    notice: Option<&'static str>,
    saving: bool,
    quit_requested: bool,
    opening: bool,
    save_requested: bool,
    operation: u64,
    secret_epoch: u64,
    local: crate::local_settings::LocalSettings,
    local_busy: bool,
    local_ready: bool,
    local_completion: Option<super::forms::Done>,
    _catalog_subscription: Subscription,
}
impl WorkspaceStore {
    pub fn new(catalog: Entity<CatalogStore>, cx: &mut Context<Self>) -> Self {
        let backend = Backend::new(cx.global::<crate::macos::LaunchProfile>().0.clone());
        let notice = if let Some(platform) = cx.try_global::<crate::macos::platform::Platform>() {
            if let Ok(backend) = &backend {
                platform.attach(backend.sessions.clone());
            }
            backend.as_ref().err().map(error_key)
        } else {
            Some("ui.platform_unavailable")
        };
        let mut result = Self {
            sync: SyncStore::default(),
            state: Default::default(),
            catalog: catalog.clone(),
            editor: None,
            backend: backend.ok(),
            connections: BTreeMap::new(),
            requests: BTreeMap::new(),
            reported_locks: Default::default(),
            pending: Vec::new(),
            notice,
            saving: false,
            quit_requested: false,
            opening: false,
            save_requested: false,
            operation: 0,
            secret_epoch: 0,
            local: Default::default(),
            local_busy: false,
            local_ready: false,
            local_completion: None,
            _catalog_subscription: cx.observe(&catalog, |_, _, cx| cx.notify()),
        };
        if let Some(backend) = &result.backend {
            let profile = backend.profile.clone();
            result.watch(
                crate::backend::background(move || {
                    crate::local_settings::LocalSettings::load(&profile)
                }),
                |store, result, _, cx| {
                    match result {
                        Ok(local) => {
                            store.catalog.update(cx, |catalog, cx| {
                                for recent in &local.recent {
                                    catalog.add_path(recent.database.clone(), recent.path.clone());
                                }
                                cx.notify();
                            });
                            if cx.has_global::<crate::macos::platform::Platform>() {
                                cx.global_mut::<crate::macos::platform::Platform>()
                                    .clipboard_seconds = local.clipboard_seconds;
                            }
                            store.local = local;
                            store.local_ready = true;
                        }
                        Err(_) => store.notice = Some("prefs_error"),
                    }
                    cx.notify();
                },
            );
        }
        result
    }
    pub fn local(&self) -> &crate::local_settings::LocalSettings {
        &self.local
    }
    pub fn idle_seconds(&self) -> u32 {
        self.backend
            .as_ref()
            .map_or(300, |b| b.sessions.policy().idle_seconds())
    }
    pub fn settings_busy(&self) -> bool {
        self.local_busy || !self.local_ready
    }
    pub fn set_idle(&mut self, seconds: u32, cx: &mut Context<Self>) {
        if self.local_busy {
            return;
        }
        let (Some(backend), Some(policy)) =
            (&self.backend, taypeer_core::SessionPolicy::new(seconds))
        else {
            return;
        };
        let profile = backend.profile.clone();
        let sessions = backend.sessions.clone();
        self.local_busy = true;
        self.watch(
            crate::backend::background(move || {
                taypeer_runtime::session::SessionSettings::save(&profile, policy)?;
                sessions.set_policy(policy);
                Ok(())
            }),
            |store, result, window, cx| {
                store.local_busy = false;
                if let Some(done) = store.local_completion.take() {
                    done(result.map_err(|_| FormError::Backend), window, cx);
                }
                if result.is_err() {
                    store.notice = Some("ui.settings_not_saved");
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    pub fn save_local(
        &mut self,
        local: crate::local_settings::LocalSettings,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy() {
            return;
        }
        let Some(backend) = &self.backend else {
            return;
        };
        let profile = backend.profile.clone();
        self.local_busy = true;
        self.watch(
            crate::backend::background(move || {
                local.save(&profile)?;
                Ok(local)
            }),
            |store, result, window, cx| {
                store.local_busy = false;
                if let Some(done) = store.local_completion.take() {
                    done(
                        result.as_ref().map(|_| ()).map_err(|_| FormError::Backend),
                        window,
                        cx,
                    );
                }
                match result {
                    Ok(local) => {
                        if cx.has_global::<crate::macos::platform::Platform>() {
                            cx.global_mut::<crate::macos::platform::Platform>()
                                .clipboard_seconds = local.clipboard_seconds;
                        }
                        store.local = local;
                    }
                    Err(_) => store.notice = Some("ui.settings_not_saved"),
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    pub fn save_local_form(
        &mut self,
        local: crate::local_settings::LocalSettings,
        done: super::forms::Done,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy() {
            done(Err(FormError::Backend), window, cx);
            return;
        }
        self.local_completion = Some(done);
        self.save_local(local, cx);
    }
    pub fn set_idle_form(
        &mut self,
        seconds: u32,
        done: super::forms::Done,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.settings_busy() || seconds == 0 {
            done(Err(FormError::InvalidNumber), window, cx);
            return;
        }
        self.local_completion = Some(done);
        self.set_idle(seconds, cx);
    }
    fn remember(&mut self, db: DatabaseId, path: PathBuf, cx: &mut Context<Self>) {
        let mut local = self.local.clone();
        local.remember(db, path);
        self.save_local(local, cx);
    }
    pub fn state(&self) -> &NavigationState {
        &self.state
    }
    pub fn catalog(&self) -> &Entity<CatalogStore> {
        &self.catalog
    }
    pub fn editor(&self) -> Option<&Entity<EditorStore>> {
        self.editor.as_ref()
    }
    pub fn connection(&self) -> Option<&Connection> {
        self.state
            .database
            .as_ref()
            .and_then(|db| self.connections.get(db))
    }
    pub fn notice(&self) -> Option<&'static str> {
        self.notice
    }
    pub fn busy(&self) -> bool {
        self.saving || self.opening || self.save_requested
    }
    pub fn secret_epoch(&self) -> u64 {
        self.secret_epoch
    }
    pub fn writable(&self, cx: &App) -> bool {
        self.state.is_unlocked()
            && self
                .state
                .database
                .as_ref()
                .and_then(|db| self.catalog.read(cx).database(db))
                .is_some_and(|db| db.writable)
    }
    pub fn set_notice(&mut self, notice: &'static str, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }
    pub fn dirty(&self, cx: &App) -> bool {
        self.editor.as_ref().is_some_and(|e| e.read(cx).dirty())
    }
    pub fn activity(&self) {
        if let Some(backend) = &self.backend {
            backend.sessions.activity().touch();
        }
    }
    pub fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.state.query != query {
            self.state.query = query;
            self.state.bookmark = None;
            self.refresh(cx);
        }
    }
    pub fn set_scope(&mut self, scope: SearchScope, cx: &mut Context<Self>) {
        self.state.scope = scope;
        self.refresh(cx);
    }
    pub fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.state.sort_by(column);
        cx.notify();
    }
    pub fn toggle_column(&mut self, column: Column, cx: &mut Context<Self>) {
        if column != Column::Title {
            if let Some(i) = self.state.columns.iter().position(|c| *c == column) {
                self.state.columns.remove(i);
            } else {
                self.state.columns.push(column);
            }
        }
        cx.notify();
    }
    pub fn move_column(&mut self, from: usize, to: usize, cx: &mut Context<Self>) {
        self.state.move_column(from, to);
        cx.notify();
    }
    pub fn watch<T: 'static>(
        &mut self,
        ticket: Ticket<T>,
        callback: impl FnOnce(&mut Self, Result<T, RuntimeError>, &mut Window, &mut Context<Self>)
        + 'static,
    ) {
        let mut callback = Some(callback);
        self.pending.push(Box::new(move |this, window, cx| {
            let Some(result) = ticket.try_take() else {
                return false;
            };
            if let Some(callback) = callback.take() {
                callback(this, result, window, cx);
            }
            true
        }));
    }
    fn watch_operation<T: 'static>(
        &mut self,
        ticket: Ticket<T>,
        callback: impl FnOnce(&mut Self, Result<T, RuntimeError>, &mut Window, &mut Context<Self>)
        + 'static,
    ) {
        let operation = self.operation;
        self.watch(ticket, move |store, result, window, cx| {
            if store.operation == operation {
                callback(store, result, window, cx);
            }
        });
    }
    /// Polling does not renew activity and never blocks on a worker.
    pub fn request_quit(&mut self, cx: &mut Context<Self>) {
        self.quit_requested = true;
        cx.notify();
    }
    pub fn poll(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(backend) = &self.backend
            && self.sync.poll(backend, &self.local.relay)
        {
            cx.notify();
        }
        if let Some(path) = self.sync.take_received() {
            self.select_path(path, window, cx);
        }
        if self.quit_requested && !self.busy() && self.state.pending.is_none() {
            self.quit_requested = false;
            self.navigate(Destination::Quit, window, cx);
        }
        let events = cx
            .try_global::<crate::macos::platform::Platform>()
            .map(|platform| {
                platform
                    .drain()
                    .into_iter()
                    .map(|event| {
                        let result = platform.copy_result(&event);
                        (event, result)
                    })
                    .collect::<Vec<_>>()
            })
            .unwrap_or_default();
        {
            for (event, result) in events {
                if let Some((success, notify)) = result {
                    if notify || !success {
                        use gpui_kit::component::notification::{Notification, NotificationType};
                        window.push_notification(
                            Notification::new()
                                .with_type(if success {
                                    NotificationType::Success
                                } else {
                                    NotificationType::Error
                                })
                                .message(super::style::tr(if success {
                                    "ui.copied"
                                } else {
                                    "ui.clipboard_error"
                                })),
                            cx,
                        );
                    }
                    continue;
                }
                match event.as_str() {
                    "sleep" | "locked" | "platform_closed" => {
                        self.sync.pause();
                        self.secret_epoch += 1;
                        self.operation += 1;
                        self.opening = false;
                        window.close_all_dialogs(cx);
                        if let Some(backend) = &self.backend {
                            backend.sessions.lock_all(if event == "sleep" {
                                LockReason::Sleep
                            } else {
                                LockReason::SystemLocked
                            });
                        }
                        cx.notify();
                    }
                    "clipboard_error" => self.notice = Some("ui.clipboard_error"),
                    _ => {}
                }
            }
        }
        let closed: Vec<_> = self
            .connections
            .iter()
            .filter(|(_, c)| !c.control.is_open())
            .map(|(db, c)| (db.clone(), c.control.status()))
            .collect();
        for (db, status) in closed {
            if self.state.unlocked.contains(&db) {
                self.hide_database(&db, window, cx);
            }
            if let Some(outcome) = status.outcome
                && self.reported_locks.insert((db.clone(), status.generation))
                && (outcome.error.is_some()
                    || outcome.draft != taypeer_runtime::session::DraftDisposition::Preserved)
            {
                self.notice = Some("ui.draft_unconfirmed");
                cx.notify();
            }
        }
        let mut updated = false;
        for connection in self.connections.values() {
            updated |= connection.take_updates();
        }
        if updated {
            self.refresh(cx);
        }
        if let Some(editor) = &self.editor
            && editor.update(cx, |editor, cx| {
                let changed = editor.poll();
                if let Some(preview) = editor.take_preview() {
                    super::images::Images::install(editor.database(), preview, cx);
                }
                if changed {
                    cx.notify();
                }
                changed
            })
        {
            cx.notify();
        }
        if self.save_requested && self.editor.as_ref().is_some_and(|e| !e.read(cx).busy()) {
            self.save(cx);
        }
        let pending = std::mem::take(&mut self.pending);
        for mut job in pending {
            if !job(self, window, cx) {
                self.pending.push(job);
            }
        }
    }
    pub fn refresh(&mut self, cx: &mut Context<Self>) {
        let targets: Vec<_> =
            if !self.state.query.is_empty() && self.state.scope == SearchScope::AllUnlocked {
                self.state.unlocked.iter().cloned().collect()
            } else {
                self.state.database.iter().cloned().collect()
            };
        for db in targets {
            let Some(connection) = self
                .connections
                .get(&db)
                .filter(|c| c.control.is_open())
                .cloned()
            else {
                continue;
            };
            let revision = self.requests.entry(db.clone()).or_default();
            *revision += 1;
            let request = *revision;
            let query = Query {
                group: if self.state.query.is_empty() {
                    self.state.group.clone()
                } else {
                    None
                },
                search: self.state.query.clone(),
                selected: if self.state.database.as_ref() == Some(&db) {
                    self.state.selected.clone()
                } else {
                    None
                },
            };
            let control = connection.control.clone();
            let generation = control.status().generation;
            self.watch(connection.query(query), move |this, result, _, cx| {
                if !control.is_open()
                    || this.requests.get(&db) != Some(&request)
                    || this
                        .connections
                        .get(&db)
                        .is_none_or(|c| c.control.status().generation != generation)
                {
                    return;
                }
                match result {
                    Ok(mut snapshot) => {
                        for preview in std::mem::take(&mut snapshot.previews) {
                            super::images::Images::install(&db, preview, cx);
                        }
                        if let Some(pending) = snapshot.pending.clone() {
                            this.state.suspended.insert(db.clone(), pending);
                        } else {
                            this.state.suspended.remove(&db);
                        }
                        this.catalog.update(cx, |catalog, cx| {
                            catalog.apply(&db, snapshot);
                            cx.notify();
                        });
                    }
                    Err(error) => this.notice = Some(error_key(&error)),
                }
                cx.notify();
            });
        }
        cx.notify();
    }
    pub fn select_path(&mut self, path: PathBuf, window: &mut Window, _cx: &mut Context<Self>) {
        let ticket = crate::backend::inspect_path(path.clone());
        self.watch(ticket, move |this, result, window, cx| match result {
            Ok(id) => {
                this.catalog.update(cx, |catalog, cx| {
                    catalog.add_path(id.clone(), path.clone());
                    cx.notify();
                });
                this.remember(id.clone(), path, cx);
                this.navigate(Destination::Database(id), window, cx);
            }
            Err(error) => this.set_notice(error_key(&error), cx),
        });
        let _ = window;
    }
    pub fn open_file(
        &mut self,
        path: PathBuf,
        password: String,
        form: Option<taypeer_services::CreateDatabase>,
        done: impl FnOnce(Result<(), RuntimeError>, &mut Window, &mut Context<Self>) + 'static,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let mut password = zeroize::Zeroizing::new(password);
        if self.busy() || self.editor.is_some() {
            self.set_notice("ui.finish_editing", cx);
            done(
                Err(RuntimeError::Service(
                    taypeer_services::ServiceError::EditorAlreadyOpen,
                )),
                window,
                cx,
            );
            return;
        }
        if cx
            .try_global::<crate::macos::platform::Platform>()
            .is_none_or(|p| !p.available())
        {
            self.set_notice("ui.platform_unavailable", cx);
            done(Err(RuntimeError::Closed), window, cx);
            return;
        }
        let Some(backend) = &self.backend else {
            self.set_notice("ui.operation_failed", cx);
            done(Err(RuntimeError::Closed), window, cx);
            return;
        };
        let ticket = backend.open(&path, std::mem::take(&mut *password), form);
        self.opening = true;
        self.operation += 1;
        self.notice = None;
        self.watch_operation(ticket, move |this, result, window, cx| {
            this.opening = false;
            match result {
                Ok(mut opened) if opened.connection.control.is_open() => {
                    let db = opened.connection.database.clone();
                    for preview in std::mem::take(&mut opened.snapshot.previews) {
                        super::images::Images::install(&db, preview, cx);
                    }
                    this.catalog.update(cx, |catalog, cx| {
                        catalog.add_path(db.clone(), path.clone());
                        catalog.apply(&db, opened.snapshot);
                        cx.notify();
                    });
                    this.remember(db.clone(), path, cx);
                    this.connections.insert(db.clone(), opened.connection);
                    this.state.unlocked.insert(db.clone());
                    this.state.select_database(db, this.catalog.read(cx));
                    this.start_sync();
                    this.refresh(cx);
                    done(Ok(()), window, cx);
                }
                Ok(_) => done(Err(RuntimeError::Closed), window, cx),
                Err(error) => {
                    this.notice = Some(error_key(&error));
                    done(Err(error), window, cx);
                }
            }
            cx.notify();
        });
        cx.notify();
    }
    pub fn unlock(&mut self, password: String, window: &mut Window, cx: &mut Context<Self>) {
        let Some(path) = self
            .state
            .database
            .as_ref()
            .and_then(|db| self.catalog.read(cx).database(db))
            .map(|db| db.path.clone())
        else {
            return;
        };
        self.open_file(path, password, None, |_, _, _| {}, window, cx);
    }
    pub fn navigate(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self.busy() || self.state.pending.is_some() {
            return;
        }
        if self.dirty(cx) {
            self.state.pending = Some(destination);
            self.unsaved(window, cx);
        } else {
            self.perform(destination, window, cx);
        }
        cx.notify();
    }
    fn unsaved(&self, window: &mut Window, cx: &mut Context<Self>) {
        let store = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let stay = store.clone();
            let discard = store.clone();
            let save = store.clone();
            dialog
                .title(tr("unsaved"))
                .child(tr("unsaved_body"))
                .close_button(false)
                .overlay_closable(false)
                .footer(
                    h_flex()
                        .gap_2()
                        .child(Button::new("stay").label(tr("stay")).on_click(
                            move |_, window, cx| {
                                let _ = stay.update(cx, |s, cx| {
                                    s.state.pending = None;
                                    cx.notify();
                                });
                                window.close_dialog(cx);
                            },
                        ))
                        .child(Button::new("discard").label(tr("discard")).on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ =
                                    discard.update(cx, |s, cx| s.discard_and_continue(window, cx));
                            },
                        ))
                        .child(Button::new("save").primary().label(tr("save")).on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = save.update(cx, |s, cx| s.save(cx));
                            },
                        )),
                )
        });
    }
    fn discard_and_continue(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(connection) = self.connection().cloned() else {
            return;
        };
        self.saving = true;
        self.operation += 1;
        if let Some(editor) = &self.editor {
            editor.update(cx, |editor, cx| {
                editor.freeze(true);
                cx.notify();
            });
        }
        self.watch_operation(
            connection.command::<()>(Command::DiscardDraft),
            |this, result, window, cx| {
                this.saving = false;
                match result {
                    Ok(()) => {
                        this.editor = None;
                        this.continue_navigation(window, cx);
                    }
                    Err(error) => {
                        if let Some(editor) = &this.editor {
                            editor.update(cx, |editor, cx| {
                                editor.fail(error);
                                cx.notify();
                            });
                        }
                        this.notice = Some(error_key(&error));
                        this.state.pending = None;
                    }
                }
                cx.notify();
            },
        );
        let _ = window;
    }
    fn continue_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(destination) = self.state.pending.take() {
            self.perform(destination, window, cx);
        }
    }
    fn perform(&mut self, destination: Destination, window: &mut Window, cx: &mut Context<Self>) {
        if self.editor.is_some() {
            self.state.pending = Some(destination);
            self.discard_and_continue(window, cx);
            return;
        }
        self.notice = None;
        if self.state.route == Route::Receive && destination != Destination::Receive {
            self.sync.pause();
        }
        match destination {
            Destination::Home => {
                self.state.route = if self.state.database.is_some() {
                    Route::Workspace
                } else {
                    Route::Welcome
                };
            }
            Destination::Devices => {
                self.state.route = Route::Devices;
                self.start_sync();
            }
            Destination::Receive => {
                self.state.route = Route::Receive;
                self.start_sync();
            }
            Destination::Database(db) => self.state.select_database(db, self.catalog.read(cx)),
            Destination::Group(group) => {
                self.state.route = Route::Workspace;
                self.state.group = Some(group);
                self.state.selected = None;
                self.state.query.clear();
                self.state.bookmark = None;
            }
            Destination::Entry(db, id) => self.state.select_entry(db, id, self.catalog.read(cx)),
            Destination::ClearEntry => self.state.selected = None,
            Destination::GroupForm { id, parent } => {
                let store = cx.entity();
                window.defer(cx, move |window, cx| {
                    super::forms::group(store, id, parent, window, cx)
                });
                return;
            }
            Destination::CreateDatabase => {
                let store = cx.entity();
                window.defer(cx, move |window, cx| {
                    super::forms::database(store, None, window, cx)
                });
                return;
            }
            Destination::CloneGroup => {
                let store = cx.entity();
                window.defer(cx, move |window, cx| {
                    super::forms::clone_group(store, window, cx)
                });
                return;
            }
            Destination::TrashGroup => {
                let store = cx.entity();
                window.defer(cx, move |window, cx| {
                    super::forms::trash_group(store, window, cx)
                });
                return;
            }
            Destination::NewEntry => {
                if self.writable(cx)
                    && let Some(group) = self.state.group.clone()
                {
                    self.start_editor(Command::BeginCreate(group), cx);
                }
                return;
            }
            Destination::CloseDatabase => {
                if let Some(db) = self.state.database.clone() {
                    self.lock(window, cx);
                    if let Some(connection) = self.connections.remove(&db)
                        && let Some(backend) = &self.backend
                    {
                        self.watch(
                            backend.close(db, connection.control),
                            |store, result, _, cx| {
                                if let Err(error) = result {
                                    store.set_notice(error_key(&error), cx);
                                }
                            },
                        );
                    }
                }
                self.state.close_database(self.catalog.read(cx));
            }
            Destination::CancelEdit => {}
            Destination::SearchResults => self.state.return_to_search(),
            Destination::RestoreRevision(revision) => {
                self.confirm_history(Some(revision), window, cx);
                return;
            }
            Destination::ClearHistory => {
                self.confirm_history(None, window, cx);
                return;
            }
            Destination::Quit => {
                if let Some(backend) = &self.backend {
                    backend.sessions.lock_all(LockReason::HostExited);
                }
                cx.quit();
                return;
            }
        }
        self.refresh(cx);
    }
    fn start_editor(&mut self, command: Command, cx: &mut Context<Self>) {
        let preserve_tab = matches!(&command, Command::BeginEdit(_));
        let Some(connection) = self.connection().cloned() else {
            return;
        };
        let view_connection = connection.clone();
        self.saving = true;
        self.operation += 1;
        self.watch_operation(
            connection.command::<()>(command),
            move |this, result, _, cx| {
                if let Err(error) = result {
                    this.saving = false;
                    this.set_notice(error_key(&error), cx);
                    return;
                }
                let connection = view_connection.clone();
                this.watch_operation(
                    connection.command::<taypeer_services::EditorView>(Command::EditorView),
                    move |this, result, _, cx| {
                        this.saving = false;
                        match result {
                            Ok(view) => {
                                this.state.suspended.remove(&connection.database);
                                this.state.group = Some(view.group.clone());
                                this.state.selected = view.entry.clone();
                                this.editor =
                                    Some(cx.new(|_| EditorStore::from_view(connection, view)));
                                if !preserve_tab {
                                    this.state.tab = EntryTab::Overview;
                                }
                            }
                            Err(error) => this.notice = Some(error_key(&error)),
                        }
                        cx.notify();
                    },
                );
            },
        );
        cx.notify();
    }
    pub fn begin_edit(&mut self, cx: &mut Context<Self>) {
        if !self.writable(cx) || self.editor.is_some() || self.busy() {
            return;
        }
        if let Some(id) = self.state.selected.clone() {
            self.start_editor(Command::BeginEdit(id), cx);
        }
    }
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.opening {
            return;
        }
        let Some(editor) = self.editor.clone() else {
            return;
        };
        editor.update(cx, |editor, cx| {
            editor.freeze(true);
            cx.notify();
        });
        if editor.read(cx).busy() {
            self.save_requested = true;
            cx.notify();
            return;
        }
        self.save_requested = false;
        if editor.read(cx).error().is_some() {
            editor.update(cx, |editor, cx| {
                editor.freeze(false);
                cx.notify();
            });
            self.set_notice("ui.operation_failed", cx);
            return;
        }
        let connection = editor.read(cx).connection().clone();
        self.saving = true;
        self.operation += 1;
        self.watch_operation(
            connection.command::<EntryId>(Command::SaveDraft),
            |this, result, window, cx| {
                this.saving = false;
                match result {
                    Ok(id) => {
                        this.editor = None;
                        this.state.selected = Some(id);
                        this.notice = Some("ui.saved");
                        this.continue_navigation(window, cx);
                        this.refresh(cx);
                    }
                    Err(error) => {
                        if let Some(editor) = &this.editor {
                            editor.update(cx, |editor, cx| {
                                editor.fail(error);
                                cx.notify();
                            });
                        }
                        this.state.pending = None;
                        this.notice = Some(error_key(&error));
                    }
                }
                cx.notify();
            },
        );
        cx.notify();
    }
    fn hide_database(&mut self, db: &DatabaseId, window: &mut Window, cx: &mut Context<Self>) {
        let active = self.state.database.as_ref() == Some(db);
        self.state.lock(db);
        super::images::Images::clear(db, cx);
        if self
            .editor
            .as_ref()
            .is_some_and(|e| e.read(cx).database() == db)
        {
            if self
                .editor
                .as_ref()
                .is_some_and(|e| e.read(cx).unacknowledged())
            {
                self.notice = Some("ui.input_unconfirmed");
            }
            self.editor = None;
        }
        self.catalog.update(cx, |catalog, cx| {
            catalog.clear(db);
            cx.notify();
        });
        if active {
            self.secret_epoch += 1;
            self.operation += 1;
            self.saving = false;
            self.opening = false;
            self.save_requested = false;
            window.close_all_dialogs(cx);
        }
        cx.notify();
    }
    pub fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.opening
            && let Some(backend) = &self.backend
        {
            // The opening worker may not have supplied its database ID yet.
            backend.sessions.lock_all(LockReason::Manual);
        }
        if let Some(db) = self.state.database.clone() {
            if let Some(connection) = self.connections.get(&db) {
                connection.control.invalidate(LockReason::Manual);
            }
            self.hide_database(&db, window, cx);
        }
    }
    pub fn restore_draft(&mut self, restore: bool, cx: &mut Context<Self>) {
        if self.busy() || self.editor.is_some() {
            return;
        }
        if restore {
            self.start_editor(Command::RestoreDraft, cx);
        } else if let Some(connection) = self.connection() {
            let db = connection.database.clone();
            let ticket = connection.command::<()>(Command::DiscardDraft);
            self.watch(ticket, move |this, result, _, cx| {
                match result {
                    Ok(()) => {
                        this.state.suspended.remove(&db);
                    }
                    Err(error) => this.notice = Some(error_key(&error)),
                }
                cx.notify();
            });
        }
    }
    pub fn after_group_removed(&mut self, cx: &mut Context<Self>) {
        self.state.group = None;
        self.state.selected = None;
        self.refresh(cx);
    }
    pub fn select_tab(&mut self, tab: EntryTab, cx: &mut Context<Self>) {
        self.state.tab = tab;
        cx.notify();
    }
    pub fn settings(&mut self, cx: &mut Context<Self>) {
        self.state.route = if self.state.route == Route::Settings {
            if self.state.database.is_some() {
                Route::Workspace
            } else {
                Route::Welcome
            }
        } else {
            Route::Settings
        };
        cx.notify();
    }
    pub fn load_revision(&mut self, revision: RevisionId, _cx: &mut Context<Self>) {
        let (Some(connection), Some(entry)) =
            (self.connection().cloned(), self.state.selected.clone())
        else {
            return;
        };
        let db = connection.database.clone();
        let preview_db = db.clone();
        self.watch(
            connection.command::<Option<taypeer_services::IconPreview>>(Command::IconPreview(
                taypeer_services::BinaryTarget::Revision {
                    entry: entry.clone(),
                    revision: revision.clone(),
                },
            )),
            move |_, result, _, cx| {
                if let Ok(Some(preview)) = result {
                    super::images::Images::install(&preview_db, preview, cx);
                    cx.notify();
                }
            },
        );
        self.watch(
            connection.command(Command::Revision {
                entry: entry.clone(),
                revision: revision.clone(),
            }),
            move |this, result, _, cx| {
                match result {
                    Ok(view) => this.catalog.update(cx, |catalog, cx| {
                        catalog.set_revision(&db, &entry, &revision, view);
                        cx.notify();
                    }),
                    Err(error) => this.notice = Some(error_key(&error)),
                }
                cx.notify();
            },
        );
    }
    fn confirm_history(
        &self,
        revision: Option<RevisionId>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if !self.writable(cx) {
            return;
        }
        let (Some(connection), Some(db), Some(entry)) = (
            self.connection().cloned(),
            self.state.database.as_ref(),
            self.state.selected.as_ref(),
        ) else {
            return;
        };
        let Some(view) = self.catalog.read(cx).entry(db, entry) else {
            return;
        };
        let Some(group) = view.group.clone() else {
            return;
        };
        let revisions: std::collections::BTreeSet<_> =
            view.revisions.iter().map(|r| r.sequence.clone()).collect();
        let entry = entry.clone();
        let store = cx.entity().downgrade();
        let Ok(operation) = taypeer_services::new_operation_id() else {
            return;
        };
        super::forms::text_form(
            if revision.is_some() {
                "ui.restore_revision"
            } else {
                "ui.clear_history"
            },
            Vec::new(),
            Box::new(move |_, _, _| {
                let command = if let Some(revision) = revision.clone() {
                    Command::RestoreRevision {
                        entry: entry.clone(),
                        revision,
                        group: group.clone(),
                        operation: operation.clone(),
                    }
                } else {
                    Command::PurgeHistory {
                        entry: entry.clone(),
                        revisions: revisions.clone(),
                        operation: operation.clone(),
                    }
                };
                let ticket = connection.command::<serde_json::Value>(command);
                let store = store.clone();
                Ok(Some(Box::new(move |done, window, cx| {
                    if let Some(store) = store.upgrade() {
                        store.update(cx, |store, _| {
                            store.watch(ticket, move |store, result, window, cx| {
                                let result = result.map(|_| ()).map_err(|_| FormError::Backend);
                                if result.is_ok() {
                                    store.refresh(cx);
                                }
                                done(result, window, cx);
                            })
                        });
                    } else {
                        done(Err(FormError::MissingObject), window, cx);
                    }
                })))
            }),
            window,
            cx,
        );
    }
}
pub(super) use crate::backend::error_key;
