//! Immediate trash and its exact captured Undo selection.
use super::*;
impl WorkspaceStore {
    pub(in crate::ui) fn trash_target(
        &mut self,
        target: taypeer_services::ObjectId,
        cx: &mut Context<Self>,
    ) {
        let Some(connection) = self.connection().cloned().filter(|_| self.writable(cx)) else {
            return;
        };
        let Ok(operation) = taypeer_services::new_operation_id() else {
            return;
        };
        let destination = match &target {
            taypeer_services::ObjectId::Entry(_) => self.state.group.clone(),
            taypeer_services::ObjectId::Group(id) => self
                .state
                .database
                .as_ref()
                .and_then(|db| self.catalog.read(cx).database(db))
                .and_then(|db| db.groups.iter().find(|group| &group.id == id))
                .and_then(|group| group.parent.clone()),
        };
        self.saving = true;
        self.watch(
            connection.command::<Vec<taypeer_services::ObjectId>>(Command::TrashObject {
                target: target.clone(),
                operation,
            }),
            move |store, result, _, cx| {
                store.saving = false;
                match result {
                    Ok(_) => {
                        store.state.selected = None;
                        if matches!(target, taypeer_services::ObjectId::Group(_)) {
                            store.state.group = None;
                        }
                        store.refresh(cx);
                        let undo_connection = connection.clone();
                        store.watch(
                            connection.command::<taypeer_services::PreparedLifecycle>(
                                Command::PrepareLifecycle {
                                    action: taypeer_services::LifecycleAction::Restore,
                                    target,
                                    destination,
                                },
                            ),
                            move |store, result, _, cx| {
                                match result {
                                    Ok(prepared) => {
                                        if let Ok(operation) = taypeer_services::new_operation_id()
                                        {
                                            store.undo =
                                                Some((undo_connection, prepared, operation));
                                            store.notice = Some("ui.trashed");
                                        }
                                    }
                                    Err(error) => store.notice = Some(error_key(&error)),
                                }
                                cx.notify();
                            },
                        );
                    }
                    Err(error) => store.notice = Some(error_key(&error)),
                }
                cx.notify();
            },
        );
    }
    /// Whether the most recent exact trash selection can be restored in its original session.
    pub fn can_undo(&self) -> bool {
        self.undo
            .as_ref()
            .is_some_and(|(connection, _, _)| connection.control.is_open())
    }
    /// Restore the captured trash selection; retries preserve the same operation identity.
    pub(super) fn undo(&mut self, _cx: &mut Context<Self>) {
        let Some((connection, prepared, operation)) = &self.undo else {
            return;
        };
        if self.busy() || !connection.control.is_open() {
            return;
        }
        let target = prepared.target.clone();
        let database = connection.database.clone();
        let ticket =
            connection.command::<Vec<taypeer_services::ObjectId>>(Command::ConfirmLifecycle {
                prepared: prepared.clone(),
                operation: operation.clone(),
            });
        self.saving = true;
        self.watch(ticket, move |store, result, _, cx| {
            store.saving = false;
            match result {
                Ok(_) => {
                    store.undo = None;
                    store
                        .state
                        .select_database(database, store.catalog.read(cx));
                    match target {
                        taypeer_services::ObjectId::Group(id) => store.state.group = Some(id),
                        taypeer_services::ObjectId::Entry(id) => store.state.selected = Some(id),
                    };
                    store.refresh(cx);
                }
                Err(error) => store.notice = Some(error_key(&error)),
            }
            cx.notify();
        });
    }
}
