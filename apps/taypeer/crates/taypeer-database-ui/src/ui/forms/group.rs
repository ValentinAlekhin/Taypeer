//! Group metadata and reviewed lifecycle actions.
use super::*;

pub(in crate::ui) fn group(
    store: Entity<WorkspaceStore>,
    id: Option<GroupId>,
    parent: Option<GroupId>,
    window: &mut Window,
    cx: &mut App,
) {
    if store.read(cx).editor().is_some() {
        store.update(cx, |store, cx| {
            store.navigate(Destination::GroupForm { id, parent }, window, cx)
        });
        return;
    }
    let state = store.read(cx);
    if !state.writable(cx) {
        return;
    }
    let Some(db) = state.state().database.as_ref() else {
        return;
    };
    let group = id.as_ref().and_then(|id| {
        state
            .catalog()
            .read(cx)
            .database(db)?
            .groups
            .iter()
            .find(|g| &g.id == id)
    });
    if group.is_some_and(|g| g.description_conflict) {
        store.update(cx, |store, cx| store.set_notice("ui.metadata_conflict", cx));
        return;
    }
    let connection = state.connection().cloned();
    let original_icon = group.map(|g| g.source_icon.clone()).unwrap_or_default();
    let original_description = group.and_then(|g| g.description.clone());
    let fields = vec![
        (
            "name",
            group.map(|g| g.name.clone()).unwrap_or_default(),
            false,
        ),
        (
            "ui.description",
            original_description.clone().unwrap_or_default(),
            false,
        ),
    ];
    let icon = group
        .map(|g| g.icon.clone())
        .unwrap_or_else(|| "folder".into());
    let operation = taypeer_services::new_operation_id().ok();
    let original_icon_name = icon.clone();
    text_form_with_icon(
        if id.is_some() {
            "edit_group"
        } else {
            "add_group"
        },
        fields,
        Box::new(move |values, _, _| {
            require_name(&values[0])?;
            let form = taypeer_services::GroupForm {
                id: id.clone(),
                parent: parent.clone(),
                name: values[0].clone(),
                description: if values[1] == original_description.as_deref().unwrap_or_default() {
                    original_description.clone()
                } else {
                    Some(values[1].clone())
                },
                icon: if values[2] == original_icon_name {
                    original_icon.clone()
                } else {
                    taypeer_core::IconRef::Lucide(
                        values[2]
                            .clone()
                            .try_into()
                            .map_err(|_| FormError::MissingObject)?,
                    )
                },
            };
            let operation = operation.clone().ok_or(FormError::Backend)?;
            let connection = connection.clone().ok_or(FormError::MissingObject)?;
            let store = store.clone();
            Ok(Some(Box::new(move |done, _, cx| {
                store.update(cx, |s, _| {
                    s.watch(
                        connection.command::<GroupId>(taypeer_runtime::Command::SaveGroup {
                            form,
                            operation,
                        }),
                        move |s, result, window, cx| match result {
                            Ok(group) => {
                                s.navigate(Destination::Group(group), window, cx);
                                done(Ok(()), window, cx);
                            }
                            Err(_) => done(Err(FormError::Backend), window, cx),
                        },
                    );
                });
            })))
        }),
        Some(icon),
        window,
        cx,
    );
}

pub(in crate::ui) fn clone_group(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut App) {
    let state = store.read(cx);
    let (Some(connection), Some(group)) =
        (state.connection().cloned(), state.state().group.clone())
    else {
        return;
    };
    let Some(source) = state
        .catalog()
        .read(cx)
        .database(&connection.database)
        .and_then(|db| db.groups.iter().find(|g| g.id == group))
    else {
        return;
    };
    let parent = source.parent.clone();
    let name = source.name.clone();
    let Ok(operation) = taypeer_services::new_operation_id() else {
        return;
    };
    text_form(
        "ui.clone_group",
        vec![("name", name, false)],
        Box::new(move |values, _, _| {
            require_name(&values[0])?;
            let ticket = connection.command::<Vec<taypeer_services::ObjectId>>(
                taypeer_runtime::Command::CloneGroup {
                    group: group.clone(),
                    parent: parent.clone(),
                    name: Some(values[0].clone()),
                    operation: operation.clone(),
                },
            );
            let store = store.clone();
            Ok(Some(Box::new(move |done, _, cx| {
                store.update(cx, |store, _| {
                    store.watch(ticket, move |store, result, window, cx| match result {
                        Ok(objects) => {
                            if let Some(taypeer_services::ObjectId::Group(group)) = objects.first()
                            {
                                store.navigate(Destination::Group(group.clone()), window, cx);
                            } else {
                                store.refresh(cx);
                            }
                            done(Ok(()), window, cx);
                        }
                        Err(_) => done(Err(FormError::Backend), window, cx),
                    })
                })
            })))
        }),
        window,
        cx,
    );
}
pub(in crate::ui) fn trash_group(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut App) {
    let state = store.read(cx);
    let (Some(connection), Some(group)) =
        (state.connection().cloned(), state.state().group.clone())
    else {
        return;
    };
    let ticket = connection.command::<taypeer_services::PreparedLifecycle>(
        taypeer_runtime::Command::PrepareLifecycle {
            action: taypeer_services::LifecycleAction::Trash,
            target: taypeer_services::ObjectId::Group(group),
            destination: None,
        },
    );
    let handle = store.clone();
    store.update(cx, |store, _| {
        store.watch(ticket, move |store, result, window, cx| {
            let prepared = match result {
                Ok(prepared) => prepared,
                Err(error) => {
                    store.set_notice(crate::ui::workspace::error_key(&error), cx);
                    return;
                }
            };
            let Ok(operation) = taypeer_services::new_operation_id() else {
                return;
            };
            text_form(
                "ui.trash_group_confirm",
                Vec::new(),
                Box::new(move |_, _, _| {
                    let ticket = connection.command::<Vec<taypeer_services::ObjectId>>(
                        taypeer_runtime::Command::ConfirmLifecycle {
                            prepared: prepared.clone(),
                            operation: operation.clone(),
                        },
                    );
                    let store = handle.clone();
                    Ok(Some(Box::new(move |done, _, cx| {
                        store.update(cx, |store, _| {
                            store.watch(ticket, move |store, result, window, cx| {
                                let result = result.map(|_| ()).map_err(FormError::Runtime);
                                if result.is_ok() {
                                    store.after_group_removed(cx);
                                }
                                done(result, window, cx);
                            })
                        })
                    })))
                }),
                window,
                cx,
            );
        })
    });
    let _ = window;
}
