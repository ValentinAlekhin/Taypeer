//! Descriptive history comes from confirmed service revisions, never reconstructed from inputs.
use super::{style::*, workspace::WorkspaceStore};
use gpui_kit::{
    component::{
        menu::{DropdownMenu, PopupMenuItem},
        *,
    },
    *,
};

pub(super) struct Row {
    pub id: taypeer_core::RevisionId,
    pub saved: i64,
    pub name: String,
    pub description: Option<String>,
}
fn version_row(row: &Row, cx: &App) -> impl IntoElement {
    let description = row.description.clone().unwrap_or_default();
    super::history::version(
        SharedString::from(format!("metadata-history-version-{}", row.id.as_str())),
        row.saved,
        v_flex()
            .gap_1()
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
                    .text_size(rems(13. / 16.))
                    .child(description),
            ),
        false,
        false,
        cx,
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
    window.open_dialog(cx, move |dialog, window, cx| {
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
                    .child(super::history::column_header(cx))
                    .children(rows.iter().rev().map(|row| version_row(row, cx))),
            )
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        icon_button("metadata-history-menu", "ellipsis", "ui.entry_actions")
                            .dropdown_menu(move |menu, _, _| {
                                let store = clear_store.clone();
                                let group = clear_group.clone();
                                let revisions = revisions.clone();
                                menu.item(
                                    PopupMenuItem::new(tr("ui.clear_history"))
                                        .disabled(revisions.is_empty())
                                        .on_click(move |_, window, cx| {
                                            confirm_purge(
                                                store.clone(),
                                                group.clone(),
                                                revisions.clone(),
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                            }),
                    )
                    .child(
                        default_button("close-history")
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
