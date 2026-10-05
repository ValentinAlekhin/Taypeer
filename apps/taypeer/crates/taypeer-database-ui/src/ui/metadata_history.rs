//! Descriptive history comes from confirmed service revisions, never reconstructed from inputs.
use super::{style::*, workspace::WorkspaceStore};
use gpui_kit::{
    component::{button::*, *},
    *,
};

pub(super) struct Row {
    pub id: taypeer_core::RevisionId,
    pub saved: i64,
    pub name: String,
    pub description: Option<String>,
}
fn version_row(row: &Row) -> impl IntoElement {
    let description = row.description.clone().unwrap_or_default();
    v_flex()
        .gap_1()
        .child(stamp(row.saved))
        .child(
            div()
                .id(SharedString::from(format!(
                    "metadata-history-name-{}",
                    row.id.as_str()
                )))
                .test_support()
                .aria_label(row.name.clone())
                .child(row.name.clone()),
        )
        .child(
            div()
                .id(SharedString::from(format!(
                    "metadata-history-description-{}",
                    row.id.as_str()
                )))
                .test_support()
                .aria_label(description.clone())
                .text_sm()
                .child(description),
        )
}
pub(super) fn show(
    store: WeakEntity<WorkspaceStore>,
    group: Option<taypeer_core::GroupId>,
    rows: Vec<Row>,
    window: &mut Window,
    cx: &mut App,
) {
    let revisions: Vec<_> = rows.iter().map(|row| row.id.clone()).collect();
    let rows = std::rc::Rc::new(rows);
    window.open_dialog(cx, move |dialog, window, _| {
        let clear_store = store.clone();
        let clear_group = group.clone();
        let revisions = revisions.clone();
        dialog
            .title(tr(if group.is_some() {
                "ui.group_history"
            } else {
                "ui.database_history"
            }))
            .width(window.rem_size() * 38.75)
            .child(
                v_flex()
                    .gap_3()
                    .children(rows.iter().rev().map(version_row)),
            )
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        Button::new("clear-metadata-history")
                            .label(tr("ui.clear_history"))
                            .disabled(revisions.is_empty())
                            .on_click(move |_, window, cx| {
                                confirm_purge(
                                    clear_store.clone(),
                                    clear_group.clone(),
                                    revisions.clone(),
                                    window,
                                    cx,
                                )
                            }),
                    )
                    .child(
                        Button::new("close-history")
                            .label(tr("back"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    ),
            )
    });
}
fn confirm_purge(
    store: WeakEntity<WorkspaceStore>,
    group: Option<taypeer_core::GroupId>,
    revisions: Vec<taypeer_core::RevisionId>,
    window: &mut Window,
    cx: &mut App,
) {
    let Ok(operation) = taypeer_services::new_operation_id() else {
        return;
    };
    super::forms::text_form(
        "ui.clear_history",
        Vec::new(),
        Box::new(move |_, _, _| {
            let store = store.clone();
            let group = group.clone();
            let revisions = revisions.clone();
            let operation = operation.clone();
            Ok(Some(Box::new(move |done, window, cx| {
                let Some(store) = store.upgrade() else {
                    done(Err(taypeer_ui::FormError::MissingObject), window, cx);
                    return;
                };
                store.update(cx, |store, cx| {
                    let Some(connection) = store.connection() else {
                        done(Err(taypeer_ui::FormError::MissingObject), window, cx);
                        return;
                    };
                    let command = if let Some(group) = group {
                        taypeer_runtime::Command::PurgeGroupHistory {
                            group,
                            revisions,
                            operation,
                        }
                    } else {
                        taypeer_runtime::Command::PurgeDatabaseHistory {
                            revisions,
                            operation,
                        }
                    };
                    store.watch(
                        connection.command::<()>(command),
                        move |store, result, window, cx| {
                            store.refresh(cx);
                            if result.is_ok() {
                                window.close_all_dialogs(cx);
                            }
                            done(result.map_err(taypeer_ui::FormError::Runtime), window, cx);
                        },
                    );
                });
            })))
        }),
        window,
        cx,
    );
}
