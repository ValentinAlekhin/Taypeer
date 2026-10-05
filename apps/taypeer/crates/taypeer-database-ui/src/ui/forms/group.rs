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
    store.update(cx, |store, cx| {
        let command = id.map_or(
            taypeer_runtime::Command::BeginCreateGroup(parent),
            taypeer_runtime::Command::BeginEditGroup,
        );
        store.open_metadata(command, cx);
    });
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
pub(in crate::ui) fn trash_group(store: Entity<WorkspaceStore>, _: &mut Window, cx: &mut App) {
    store.update(cx, |store, cx| {
        if let Some(group) = store.state().group.clone() {
            store.trash_target(taypeer_services::ObjectId::Group(group), cx);
        }
    });
}
