//! Group tree with its own keyboard interaction and expansion state.

use super::{forms, style::*, workspace::WorkspaceStore};
use crate::ui_state::*;
use gpui_kit::component::button::ButtonVariants;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        list::ListItem,
        menu::{DropdownMenu, PopupMenuItem},
        tree::*,
        *,
    },
    *,
};
use std::collections::BTreeSet;

pub(super) struct Sidebar {
    store: Entity<WorkspaceStore>,
    tree: Entity<TreeState>,
    source: Option<(DatabaseId, u64)>,
    database: Option<DatabaseId>,
    selection: Option<GroupId>,
    expanded: BTreeSet<String>,
    _subscriptions: Vec<Subscription>,
}
impl Sidebar {
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.observe_in(&tree, window, |this, tree, window, cx| {
                let group = tree
                    .read(cx)
                    .selected_item()
                    .map(|item| GroupId::new(item.id.as_str()));
                if let Some(group) = group
                    && this.store.read(cx).state().group.clone() != Some(group.clone())
                {
                    this.store.update(cx, |store, cx| {
                        store.navigate(Destination::Group(group.clone()), window, cx)
                    });
                }
            }),
            cx.subscribe(&tree, |this, _, event: &TreeEvent, cx| {
                match event {
                    TreeEvent::Expanded(id) => {
                        this.expanded.insert(id.to_string());
                    }
                    TreeEvent::Collapsed(id) => {
                        this.expanded.remove(id.as_str());
                    }
                }
                cx.notify();
            }),
        ];
        Self {
            store,
            tree,
            source: None,
            database: None,
            selection: None,
            expanded: BTreeSet::new(),
            _subscriptions: subscriptions,
        }
    }
    fn sync(&mut self, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let Some(db) = store.state().database.clone() else {
            return;
        };
        let catalog = store.catalog().read(cx);
        let source = (db.clone(), catalog.version());
        if self.source.as_ref() != Some(&source) {
            let Some(database) = catalog.database(&db) else {
                return;
            };
            fn items(
                groups: &[Group],
                parent: Option<GroupId>,
                expanded: &BTreeSet<String>,
                initial: bool,
            ) -> Vec<TreeItem> {
                groups
                    .iter()
                    .filter(|group| group.parent == parent)
                    .map(|group| {
                        let id = group.id.as_str().to_owned();
                        TreeItem::new(id.clone(), group.name.clone())
                            .expanded(initial || expanded.contains(&id))
                            .children(items(groups, Some(group.id.clone()), expanded, initial))
                    })
                    .collect()
            }
            let initial = self.database.as_ref() != Some(&db);
            self.database = Some(db.clone());
            if initial {
                self.expanded = database
                    .groups
                    .iter()
                    .map(|g| g.id.as_str().to_owned())
                    .collect();
            }
            let items = items(&database.groups, None, &self.expanded, initial);
            self.tree.update(cx, |tree, cx| tree.set_items(items, cx));
            self.source = Some(source);
        }
        let group = self.store.read(cx).state().group.clone();
        let changed = self.selection != group;
        self.selection = group.clone();
        if let Some(group) = group {
            let key: SharedString = group.as_str().to_owned().into();
            let index = self.tree.read(cx).index_of(&key);
            if changed && index.is_none() {
                let item = TreeItem::new(key, "");
                self.tree
                    .update(cx, |tree, cx| tree.set_selected_item(Some(&item), cx));
            } else if self.tree.read(cx).selected_index() != index {
                self.tree
                    .update(cx, |tree, cx| tree.set_selected_index(index, cx));
            }
        }
    }
}
impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        self.sync(cx);
        let sidebar = cx.entity();
        let tree = self.tree.clone();
        let store = self.store.clone();
        let menu_store = self.store.clone();
        let context_store = self.store.clone();
        let empty_tree = self.tree.read(cx).entry(0).is_none();
        v_flex()
            .size_full()
            .border_r_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .h(rems(2.75))
                    .flex_shrink_0()
                    .justify_end()
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        icon_button("group-menu", "ellipsis", "ui.group_actions").dropdown_menu(
                            move |menu, _, cx| {
                                let state = menu_store.read(cx);
                                let parent = state.state().group.clone();
                                let child = menu_store.clone();
                                let edit = menu_store.clone();
                                let edit_parent = parent.clone();
                                let clone = menu_store.clone();
                                let delete = menu_store.clone();
                                let writable = state.writable(cx) && parent.is_some();
                                menu.item(
                                    PopupMenuItem::new(tr("add_child"))
                                        .disabled(parent.is_none())
                                        .on_click(move |_, window, cx| {
                                            forms::group(
                                                child.clone(),
                                                None,
                                                parent.clone(),
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new(tr("edit_group"))
                                        .disabled(edit_parent.is_none())
                                        .on_click(move |_, window, cx| {
                                            forms::group(
                                                edit.clone(),
                                                edit_parent.clone(),
                                                None,
                                                window,
                                                cx,
                                            )
                                        }),
                                )
                                .separator()
                                .item(
                                    PopupMenuItem::new(tr("ui.clone_group"))
                                        .disabled(!writable)
                                        .on_click(move |_, window, cx| {
                                            clone.update(cx, |store, cx| {
                                                store.navigate(Destination::CloneGroup, window, cx)
                                            })
                                        }),
                                )
                                .item(
                                    PopupMenuItem::new(tr("ui.delete_group"))
                                        .disabled(!writable)
                                        .on_click(move |_, window, cx| {
                                            delete.update(cx, |store, cx| {
                                                store.navigate(Destination::TrashGroup, window, cx)
                                            })
                                        }),
                                )
                            },
                        ),
                    )
                    .child(
                        icon_button("add-group", "folder-plus", "add_group").on_click(cx.listener(
                            |this, _, window, cx| {
                                forms::group(this.store.clone(), None, None, window, cx)
                            },
                        )),
                    ),
            )
            .child(div().flex_1().min_h_0().p_1().child(if empty_tree {
                v_flex()
                    .p_3()
                    .gap_3()
                    .child(tr("empty_groups"))
                    .child(
                        gpui_kit::component::button::Button::new("empty-add-group")
                            .label(tr("add_group"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                forms::group(this.store.clone(), None, None, window, cx)
                            })),
                    )
                    .into_any_element()
            } else {
                Tree::new(&self.tree, move |ix, entry, selected, _, cx| {
                    let target = Some(GroupId::new(entry.item().id.as_str()));
                    let store_click = store.clone();
                    let select_tree = tree.clone();
                    let toggle_sidebar = sidebar.clone();
                    let toggle_id = entry.item().id.to_string();
                    let state = store.read(cx);
                    let group = state
                        .state()
                        .database
                        .as_ref()
                        .and_then(|db| state.catalog().read(cx).database(db))
                        .and_then(|db| db.groups.iter().find(|g| Some(g.id.clone()) == target));
                    let count = group.map_or(0, |g| g.entry_count);
                    ListItem::new(("group", ix))
                        .h(rems(2.25))
                        .px_2()
                        .selected(selected)
                        .child(
                            h_flex()
                                .gap_2()
                                .pl(rems(entry.depth() as f32))
                                .child(
                                    div()
                                        .id(("expand-group", ix))
                                        .w_4()
                                        .flex_shrink_0()
                                        .on_mouse_down(MouseButton::Left, |_, _, cx| {
                                            cx.stop_propagation()
                                        })
                                        .when(entry.is_folder(), |el| {
                                            el.on_click(move |_, _, cx| {
                                                cx.stop_propagation();
                                                toggle_sidebar.update(cx, |sidebar, cx| {
                                                    if !sidebar.expanded.remove(&toggle_id) {
                                                        sidebar.expanded.insert(toggle_id.clone());
                                                    }
                                                    sidebar.source = None;
                                                    cx.notify();
                                                });
                                            })
                                            .child(
                                                icon(if entry.is_expanded() {
                                                    "chevron-down"
                                                } else {
                                                    "chevron-right"
                                                }),
                                            )
                                        }),
                                )
                                .child(super::images::stored_icon(
                                    group.map_or("folder", |g| g.icon.as_str()),
                                    group.and_then(|g| g.icon_blob.as_ref()),
                                    cx,
                                ))
                                .child(div().flex_1().truncate().child(entry.item().label.clone()))
                                .child(
                                    div()
                                        .text_xs()
                                        .text_color(cx.theme().muted_foreground)
                                        .child(count.to_string()),
                                ),
                        )
                        .on_mouse_down(MouseButton::Left, move |_, window, cx| {
                            cx.stop_propagation();
                            select_tree.update(cx, |tree, cx| tree.focus(window, cx));
                            if let Some(group) = &target {
                                store_click.update(cx, |store, cx| {
                                    store.navigate(Destination::Group(group.clone()), window, cx)
                                });
                            }
                        })
                })
                .context_menu(move |_, entry, menu, _, _| {
                    let id = Some(GroupId::new(entry.item().id.as_str()));
                    let edit = context_store.clone();
                    let child = context_store.clone();
                    let child_id = id.clone();
                    menu.item(PopupMenuItem::new(tr("edit_group")).on_click(
                        move |_, window, cx| {
                            forms::group(edit.clone(), id.clone(), None, window, cx)
                        },
                    ))
                    .item(
                        PopupMenuItem::new(tr("add_child")).on_click(move |_, window, cx| {
                            forms::group(child.clone(), None, child_id.clone(), window, cx)
                        }),
                    )
                })
                .into_any_element()
            }))
            .child(
                v_flex()
                    .p_2()
                    .gap_1()
                    .border_t_1()
                    .border_color(cx.theme().border)
                    .child(
                        gpui_kit::component::button::Button::new("trash")
                            .ghost()
                            .justify_start()
                            .icon(icon("trash-2"))
                            .label(tr("ui.trash"))
                            .tooltip(tr("ui.next_stage"))
                            .disabled(true),
                    )
                    .child(
                        gpui_kit::component::button::Button::new("devices")
                            .ghost()
                            .justify_start()
                            .icon(icon("laptop"))
                            .label(tr("ui.devices"))
                            .tooltip(tr("ui.next_stage"))
                            .disabled(true),
                    ),
            )
    }
}
