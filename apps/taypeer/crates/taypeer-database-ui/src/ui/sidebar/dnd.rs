//! Tree gestures carry stable identities; the service owns placement and saving.
use super::*;
use taypeer_services::SiblingPosition;

#[derive(Clone)]
pub(super) struct GroupDrag {
    database: DatabaseId,
    group: GroupId,
    epoch: u64,
}

struct DragPreview {
    store: WeakEntity<WorkspaceStore>,
    drag: GroupDrag,
}
impl Render for DragPreview {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let label = self.store.upgrade().and_then(|store| {
            let store = store.read(cx);
            if !current_session(&self.drag, store, cx) {
                return None;
            }
            store
                .catalog()
                .read(cx)
                .database(&self.drag.database)?
                .groups
                .iter()
                .find(|group| group.id == self.drag.group)
                .map(|group| SharedString::from(group.name.clone()))
        });
        let Some(label) = label else {
            return div().into_any_element();
        };
        h_flex()
            .id("group-drag-preview")
            .test_support()
            .px_3()
            .py_1()
            .gap_2()
            .rounded_md()
            .bg(cx.theme().title_bar)
            .text_color(cx.theme().foreground)
            .border_1()
            .border_color(cx.theme().border)
            .child(icon("folder"))
            .child(label)
            .into_any_element()
    }
}

#[derive(Clone, Copy)]
enum DropZone {
    Before,
    Inside,
    After,
}

fn current_session(drag: &GroupDrag, store: &WorkspaceStore, cx: &App) -> bool {
    store.state().database.as_ref() == Some(&drag.database)
        && store.secret_epoch() == drag.epoch
        && store.writable(cx)
        && store
            .connection()
            .is_some_and(|connection| connection.control.is_open())
}

fn accepts(drag: &GroupDrag, target: Option<&GroupId>, store: &WorkspaceStore, cx: &App) -> bool {
    if !current_session(drag, store, cx) || store.busy() {
        return false;
    }
    let catalog = store.catalog().read(cx);
    let Some(database) = catalog.database(&drag.database) else {
        return false;
    };
    if !database.groups.iter().any(|group| group.id == drag.group) {
        return false;
    }
    // Suppress the invalid affordance; authoritative cycle checks stay in the service.
    let mut ancestor = target.cloned();
    for _ in 0..=database.groups.len() {
        let Some(id) = ancestor else {
            return true;
        };
        if id == drag.group {
            return false;
        }
        let Some(group) = database.groups.iter().find(|group| group.id == id) else {
            return false;
        };
        ancestor = group.parent.clone();
    }
    false
}

fn destination(
    drag: &GroupDrag,
    target: &GroupId,
    zone: DropZone,
    store: &WorkspaceStore,
    cx: &App,
) -> Option<Destination> {
    if !accepts(drag, Some(target), store, cx) {
        return None;
    }
    let catalog = store.catalog().read(cx);
    let group = catalog
        .database(&drag.database)?
        .groups
        .iter()
        .find(|group| &group.id == target)?;
    let (parent, position) = match zone {
        DropZone::Inside => (Some(target.clone()), SiblingPosition::Last),
        DropZone::Before => (
            group.parent.clone(),
            SiblingPosition::Before(target.clone()),
        ),
        DropZone::After => (group.parent.clone(), SiblingPosition::After(target.clone())),
    };
    Some(Destination::MoveGroup {
        database: drag.database.clone(),
        epoch: drag.epoch,
        group: drag.group.clone(),
        parent,
        position,
    })
}

fn zone(
    target: &GroupId,
    zone: DropZone,
    store: &Entity<WorkspaceStore>,
    sidebar: &Entity<Sidebar>,
) -> AnyElement {
    let name = match zone {
        DropZone::Before => "before",
        DropZone::Inside => "into",
        DropZone::After => "after",
    };
    let target_paint = target.clone();
    let paint_store = store.clone();
    let target_drop = target.clone();
    let drop_store = store.clone();
    let sidebar = sidebar.downgrade();
    div()
        .id(SharedString::from(format!(
            "group-drop-{name}-{}",
            target.as_str()
        )))
        .test_support()
        .absolute()
        .left_0()
        .right_0()
        .when(matches!(zone, DropZone::Inside), |el| el.top_0().bottom_0())
        .when(matches!(zone, DropZone::Before), |el| {
            el.top_0().h(rems(0.5))
        })
        .when(matches!(zone, DropZone::After), |el| {
            el.bottom_0().h(rems(0.5))
        })
        .drag_over::<GroupDrag>(move |style, drag, _, cx| {
            if destination(drag, &target_paint, zone, paint_store.read(cx), cx).is_none() {
                return style;
            }
            match zone {
                DropZone::Inside => style.border_1().rounded_md(),
                DropZone::Before => style.border_t_2(),
                DropZone::After => style.border_b_2(),
            }
            .border_color(cx.theme().ring)
        })
        .on_drop(move |drag: &GroupDrag, window, cx| {
            cx.stop_propagation();
            let destination = destination(drag, &target_drop, zone, drop_store.read(cx), cx);
            if let Some(destination) = destination {
                if matches!(zone, DropZone::Inside)
                    && let Some(sidebar) = sidebar.upgrade()
                {
                    sidebar.update(cx, |sidebar, cx| {
                        sidebar.expanded.insert(target_drop.as_str().to_owned());
                        sidebar.source = None;
                        sidebar.sync(cx);
                        cx.notify();
                    });
                }
                drop_store.update(cx, |store, cx| store.navigate(destination, window, cx));
            }
        })
        .into_any_element()
}

pub(super) fn row(
    row: ListItem,
    group: &GroupId,
    store: &Entity<WorkspaceStore>,
    sidebar: &Entity<Sidebar>,
    cx: &App,
) -> ListItem {
    let state = store.read(cx);
    let row = row
        // The full-row target is below the two narrow insertion targets.
        .child(zone(group, DropZone::Inside, store, sidebar))
        .child(zone(group, DropZone::Before, store, sidebar))
        .child(zone(group, DropZone::After, store, sidebar));
    let Some(database) = state.state().database.clone() else {
        return row;
    };
    let preview_store = store.downgrade();
    row.when(state.writable(cx) && !state.busy(), |row| {
        row.on_drag(
            GroupDrag {
                database,
                group: group.clone(),
                epoch: state.secret_epoch(),
            },
            move |drag, _, _, cx| {
                cx.stop_propagation();
                cx.new(|_| DragPreview {
                    store: preview_store.clone(),
                    drag: drag.clone(),
                })
            },
        )
    })
}

fn drop_root(drag: &GroupDrag, store: &Entity<WorkspaceStore>, window: &mut Window, cx: &mut App) {
    cx.stop_propagation();
    if accepts(drag, None, store.read(cx), cx) {
        let destination = Destination::MoveGroup {
            database: drag.database.clone(),
            epoch: drag.epoch,
            group: drag.group.clone(),
            parent: None,
            position: SiblingPosition::Last,
        };
        store.update(cx, |store, cx| store.navigate(destination, window, cx));
    }
}

pub(super) fn viewport(content: impl IntoElement, store: &Entity<WorkspaceStore>) -> AnyElement {
    let store = store.clone();
    tree_viewport(content)
        .on_drop(move |drag: &GroupDrag, window, cx| drop_root(drag, &store, window, cx))
        .into_any_element()
}

pub(super) fn root(store: &Entity<WorkspaceStore>) -> AnyElement {
    let paint_store = store.clone();
    let drop_store = store.clone();
    div()
        .id("group-drop-root")
        .test_support()
        .w_full()
        .h(rems(2.))
        .flex_shrink_0()
        .drag_over::<GroupDrag>(move |style, drag, _, cx| {
            if accepts(drag, None, paint_store.read(cx), cx) {
                style.border_b_2().border_color(cx.theme().ring)
            } else {
                style
            }
        })
        .on_drop(move |drag: &GroupDrag, window, cx| {
            drop_root(drag, &drop_store, window, cx);
        })
        .into_any_element()
}
