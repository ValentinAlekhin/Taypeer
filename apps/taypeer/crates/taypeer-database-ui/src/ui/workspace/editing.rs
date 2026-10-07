//! Entry/metadata editor lifetime and acceptance of generation-scoped replies.
use super::*;
impl WorkspaceStore {
    pub(super) fn start_editor(&mut self, command: Command, cx: &mut Context<Self>) {
        let preserve_tab = matches!(&command, Command::BeginEdit(_));
        let Some(connection) = self.connection().cloned() else {
            return;
        };
        let view_connection = connection.clone();
        self.saving = true;
        self.operation += 1;
        self.watch_operation(
            connection.command::<serde_json::Value>(command),
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
    pub(in crate::ui) fn open_metadata(&mut self, command: Command, cx: &mut Context<Self>) {
        let Some(connection) = self.connection().cloned().filter(|_| self.writable(cx)) else {
            return;
        };
        let ticket = connection.command::<taypeer_services::MetadataDraftView>(command);
        self.saving = true;
        self.operation += 1;
        self.watch_operation(ticket, move |store, result, window, cx| {
            store.saving = false;
            match result {
                Ok(view) => {
                    store.metadata = Some(super::super::forms::metadata::show(
                        connection, view, window, cx,
                    ))
                }
                Err(error) => store.set_notice(error_key(&error), cx),
            }
            cx.notify();
        });
    }
    pub(super) fn poll_metadata(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(form) = self.metadata.clone() else {
            return;
        };
        form.update(cx, |form, cx| {
            if form.poll() {
                cx.notify();
            }
        });
        if form.update(cx, |form, _| form.take_confirmed()) {
            self.refresh(cx);
            if form.read(cx).closing() && form.read(cx).settled() {
                if let taypeer_services::DraftTarget::Group(group) = form.read(cx).target() {
                    self.state.group = Some(group.clone());
                }
                self.discard_requested = true;
            } else {
                form.update(cx, |form, _| form.stay());
                if self.navigation_waiting_for_save
                    && self.state.pending.is_some()
                    && !self.discard_requested
                {
                    self.confirm_navigation(window, cx);
                } else if !self.discard_requested {
                    self.state.pending = None;
                }
            }
            self.navigation_waiting_for_save = false;
        }
        if form.update(cx, |form, _| form.take_failed()) && !self.discard_requested {
            self.state.pending = None;
            self.navigation_waiting_for_save = false;
        }
        if form.update(cx, |form, _| form.take_discard()) {
            self.cancel_edit(cx);
        } else if form.update(cx, |form, _| form.take_leave()) {
            self.navigate(Destination::CancelEdit, window, cx);
        }
    }
    pub(super) fn resume_active_draft(&mut self, _cx: &mut Context<Self>) {
        let Some(connection) = self.connection().cloned() else {
            return;
        };
        self.watch(
            connection.command::<Option<taypeer_services::DraftIdentity>>(Command::ActiveDraft),
            |store, result, _, cx| {
                if let Ok(Some(draft)) = result {
                    store.resume_draft(draft.draft, cx);
                }
            },
        );
    }
    pub(super) fn resume_draft(&mut self, draft: taypeer_core::DraftId, _cx: &mut Context<Self>) {
        let Some(connection) = self.connection().cloned() else {
            return;
        };
        self.saving = true;
        self.operation += 1;
        self.watch_operation(
            connection.command::<taypeer_services::DraftIdentity>(Command::ResumeDraft(draft)),
            move |store, result, _, cx| {
                store.saving = false;
                match result {
                    Ok(identity)
                        if matches!(
                            identity.target,
                            taypeer_services::DraftTarget::Entry(_)
                                | taypeer_services::DraftTarget::NewEntry { .. }
                        ) =>
                    {
                        store.start_editor(Command::ResumeDraft(identity.draft), cx)
                    }
                    Ok(identity) => store.open_metadata(Command::MetadataDraft(identity.draft), cx),
                    Err(error) => store.set_notice(error_key(&error), cx),
                }
            },
        );
    }
    pub(in crate::ui) fn metadata_history(
        &mut self,
        group: Option<GroupId>,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection().cloned() else {
            return;
        };
        let store = cx.entity().downgrade();
        if let Some(group) = group {
            self.watch(
                connection.command::<Vec<taypeer_core::SavedGroupRevision>>(Command::GroupHistory(
                    group.clone(),
                )),
                move |this, result, window, cx| match result {
                    Ok(rows) => super::super::metadata_history::show(
                        store,
                        Some(group),
                        rows.into_iter()
                            .map(|revision| super::super::metadata_history::Row {
                                id: revision.id,
                                saved: revision.saved_at,
                                name: revision.snapshot.group.name,
                                description: revision.snapshot.description,
                            })
                            .collect(),
                        window,
                        cx,
                    ),
                    Err(error) => this.set_notice(error_key(&error), cx),
                },
            );
        } else {
            self.watch(
                connection
                    .command::<Vec<taypeer_core::SavedDatabaseRevision>>(Command::DatabaseHistory),
                move |this, result, window, cx| match result {
                    Ok(rows) => super::super::metadata_history::show(
                        store,
                        None,
                        rows.into_iter()
                            .map(|revision| super::super::metadata_history::Row {
                                id: revision.id,
                                saved: revision.saved_at,
                                name: revision.snapshot.name,
                                description: revision.snapshot.description,
                            })
                            .collect(),
                        window,
                        cx,
                    ),
                    Err(error) => this.set_notice(error_key(&error), cx),
                },
            );
        }
    }
    /// Begin editing the selected entry when the session permits it.
    pub fn begin_edit(&mut self, cx: &mut Context<Self>) {
        if !self.writable(cx) || self.editor.is_some() || self.busy() {
            return;
        }
        if let Some(id) = self.state.selected.clone() {
            self.start_editor(Command::BeginEdit(id), cx);
        }
    }
    /// Save exactly the current queued input; close only after durable confirmation without new input.
    pub fn save(&mut self, cx: &mut Context<Self>) {
        if self.saving || self.opening || self.discard_requested {
            return;
        }
        if let Some(form) = &self.metadata {
            form.update(cx, |form, cx| {
                form.request_save(cx);
                cx.notify();
            });
            if !form.read(cx).closing() {
                self.state.pending = None;
            }
        } else if let Some(editor) = &self.editor
            && !editor.update(cx, |editor, cx| {
                let started = editor.start_snapshot();
                if started {
                    cx.notify();
                }
                started
            })
            && !editor.read(cx).saving()
        {
            self.state.pending = None;
        }
        cx.notify();
    }
}
