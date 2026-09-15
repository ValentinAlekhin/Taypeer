//! Coordinates window transitions; presentation components never call a backend.

use super::style::tr;
use crate::ui_state::*;
use gpui_kit::{
    component::{button::*, *},
    *,
};

pub(super) struct WorkspaceStore {
    state: NavigationState,
    catalog: Entity<CatalogStore>,
    editor: Option<Entity<EditorStore>>,
    notice: Option<&'static str>,
    _catalog_subscription: Subscription,
}

impl WorkspaceStore {
    pub fn new(catalog: Entity<CatalogStore>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&catalog, |_, _, cx| cx.notify());
        Self {
            state: NavigationState::default(),
            catalog,
            editor: None,
            notice: None,
            _catalog_subscription: subscription,
        }
    }
    pub fn state(&self) -> &NavigationState {
        &self.state
    }
    pub fn notice(&self) -> Option<&'static str> {
        self.notice
    }
    pub fn set_notice(&mut self, notice: &'static str, cx: &mut Context<Self>) {
        self.notice = Some(notice);
        cx.notify();
    }
    pub fn set_query(&mut self, query: String, cx: &mut Context<Self>) {
        if self.state.query != query {
            self.state.query = query;
            self.state.bookmark = None;
            cx.notify();
        }
    }
    pub fn set_scope(&mut self, scope: SearchScope, cx: &mut Context<Self>) {
        self.state.scope = scope;
        cx.notify();
    }
    pub fn sort_by(&mut self, column: Column, cx: &mut Context<Self>) {
        self.state.sort_by(column);
        cx.notify();
    }
    pub fn toggle_column(&mut self, column: Column, cx: &mut Context<Self>) {
        if column == Column::Title {
            return;
        }
        if let Some(index) = self.state.columns.iter().position(|c| *c == column) {
            self.state.columns.remove(index);
        } else {
            self.state.columns.push(column);
        }
        cx.notify();
    }
    pub fn open_created(&mut self, db: DatabaseId, window: &mut Window, cx: &mut Context<Self>) {
        self.state.unlocked.insert(db);
        self.navigate(Destination::Database(db), window, cx);
    }
    pub fn catalog(&self) -> &Entity<CatalogStore> {
        &self.catalog
    }
    pub fn editor(&self) -> Option<&Entity<EditorStore>> {
        self.editor.as_ref()
    }
    pub fn dirty(&self, cx: &App) -> bool {
        self.editor
            .as_ref()
            .is_some_and(|editor| editor.read(cx).dirty())
    }

    pub fn navigate(
        &mut self,
        destination: Destination,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        // A pending destination owns the one unsaved-changes dialog.
        if self.state.pending.is_some() {
            return;
        }
        if let Destination::Entry(db, id) = destination
            && self.state.database == Some(db)
            && self.state.selected == Some(id)
        {
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
            let save = store.clone();
            let discard = store.clone();
            let stay = store.clone();
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
                                let _ = stay.update(cx, |this, cx| {
                                    this.state.pending = None;
                                    cx.notify();
                                });
                                window.close_dialog(cx);
                            },
                        ))
                        .child(Button::new("discard").label(tr("discard")).on_click(
                            move |_, window, cx| {
                                window.close_dialog(cx);
                                let _ = discard.update(cx, |this, cx| {
                                    this.editor = None;
                                    this.continue_navigation(window, cx);
                                });
                            },
                        ))
                        .child(Button::new("save").primary().label(tr("save")).on_click(
                            move |_, window, cx| {
                                let _ = save.update(cx, |this, cx| {
                                    if this.save(cx) {
                                        window.close_dialog(cx);
                                        this.continue_navigation(window, cx);
                                    } else {
                                        window.close_dialog(cx);
                                        this.state.pending = None;
                                    }
                                });
                            },
                        )),
                )
                .on_cancel({
                    let store = store.clone();
                    move |_, _, cx| {
                        let _ = store.update(cx, |this, cx| {
                            this.state.pending = None;
                            cx.notify();
                        });
                        true
                    }
                })
        });
    }

    fn continue_navigation(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(destination) = self.state.pending.take() {
            self.perform(destination, window, cx);
        }
        cx.notify();
    }

    fn perform(&mut self, destination: Destination, window: &mut Window, cx: &mut Context<Self>) {
        self.editor = None;
        self.notice = None;
        match destination {
            Destination::Database(db) => self.state.select_database(db, self.catalog.read(cx)),
            Destination::Group(group) => {
                if !self.state.is_unlocked() {
                    return;
                }
                self.state.route = Route::Workspace;
                self.state.group = Some(group);
                self.state.selected = None;
                self.state.query.clear();
                self.state.bookmark = None;
            }
            Destination::Entry(db, entry) => {
                self.state.select_entry(db, entry, self.catalog.read(cx))
            }
            Destination::NewEntry => {
                if !self.state.is_unlocked() {
                    return;
                }
                if let (Some(db), Some(group)) = (self.state.database, self.state.group) {
                    self.editor = Some(cx.new(|_| EditorStore::new(db, group)));
                    self.state.selected = None;
                    self.state.tab = EntryTab::Overview;
                }
            }
            Destination::CloseDatabase => self.state.close_database(self.catalog.read(cx)),
            Destination::CancelEdit => {}
            Destination::SearchResults => self.state.return_to_search(),
            Destination::RestoreRevision(sequence) => {
                self.confirm_history(Some(sequence), window, cx)
            }
            Destination::ClearHistory => self.confirm_history(None, window, cx),
            Destination::Quit => window.remove_window(),
        }
        cx.notify();
    }

    pub fn begin_edit(&mut self, cx: &mut Context<Self>) {
        if !self.state.is_unlocked() || self.editor.is_some() {
            return;
        }
        if let (Some(db), Some(id)) = (self.state.database, self.state.selected)
            && let Some(entry) = self.catalog.read(cx).entry(db, id)
        {
            let draft = EditorStore::existing(db, entry);
            self.editor = Some(cx.new(|_| draft));
            cx.notify();
        }
    }

    pub fn save(&mut self, cx: &mut Context<Self>) -> bool {
        let Some(editor) = self.editor.clone() else {
            return true;
        };
        if editor
            .update(cx, |editor, cx| {
                let result = editor.validate();
                cx.notify();
                result
            })
            .is_err()
        {
            cx.notify();
            return false;
        }
        let draft = editor.read(cx).clone();
        let result = self.catalog.update(cx, |catalog, cx| {
            let result = catalog.save_entry(
                draft.database(),
                draft.group(),
                draft.entry(),
                draft.content().clone(),
            );
            if result.is_ok() {
                cx.notify();
            }
            result
        });
        match result {
            Ok(id) => {
                self.state.selected = Some(id);
                self.editor = None;
                self.notice = Some("ui.saved_memory");
                cx.notify();
                true
            }
            Err(error) => {
                self.notice = Some(error.key());
                cx.notify();
                false
            }
        }
    }

    pub fn unlock(&mut self, cx: &mut Context<Self>) {
        if let Some(db) = self.state.database {
            self.state.unlocked.insert(db);
        }
        cx.notify();
    }

    pub fn lock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let draft = self.editor.take().map(|editor| editor.read(cx).clone());
        self.state.lock(draft);
        self.notice = None;
        window.close_all_dialogs(cx);
        cx.notify();
    }

    pub fn restore_draft(&mut self, restore: bool, cx: &mut Context<Self>) {
        if let Some(db) = self.state.database
            && let Some(draft) = self.state.suspended.remove(&db)
            && restore
        {
            self.state.selected = draft.entry();
            self.state.group = Some(draft.group());
            self.editor = Some(cx.new(|_| draft));
        }
        cx.notify();
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

    fn confirm_history(&self, sequence: Option<u64>, window: &mut Window, cx: &mut Context<Self>) {
        let (Some(db), Some(id)) = (self.state.database, self.state.selected) else {
            return;
        };
        let store = cx.entity().downgrade();
        window.open_dialog(cx, move |dialog, _, _| {
            let store = store.clone();
            dialog
                .title(tr(if sequence.is_some() {
                    "ui.restore_revision"
                } else {
                    "ui.clear_history"
                }))
                .child(tr("ui.history_confirm"))
                .footer(super::style::dialog_actions())
                .on_ok(move |_, _, cx| {
                    let _ = store.update(cx, |this, cx| {
                        let result = this.catalog.update(cx, |catalog, cx| {
                            let result = if let Some(sequence) = sequence {
                                catalog.restore_revision(db, id, sequence)
                            } else {
                                catalog.clear_history(db, id)
                            };
                            if result.is_ok() {
                                cx.notify();
                            }
                            result
                        });
                        this.notice = Some(result.err().map_or("ui.saved_memory", FormError::key));
                        cx.notify();
                    });
                    true
                })
        });
    }
}
