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

mod dnd;

pub(super) struct GroupPresentation {
    pub label: SharedString,
    pub count: usize,
    pub depth: usize,
    pub selected: bool,
}

pub(super) fn group_row(
    id: impl Into<ElementId>,
    group: GroupPresentation,
    group_icon: impl IntoElement,
    expand: impl IntoElement,
    cx: &App,
) -> ListItem {
    ListItem::new(id)
        .h(rems(2.))
        .px_2()
        .text_size(rems(12. / 16.))
        .aria_selected(group.selected)
        .selected(group.selected)
        .child(
            h_flex()
                .gap_2()
                .pl(rems(group.depth as f32))
                .child(expand)
                .child(group_icon)
                .child(div().flex_1().truncate().child(group.label))
                .child(
                    div()
                        .text_size(rems(11. / 16.))
                        .text_color(cx.theme().muted_foreground)
                        .child(group.count.to_string()),
                ),
        )
}

pub(super) fn toolbar(actions: impl IntoElement, cx: &App) -> AnyElement {
    h_flex()
        .id("sidebar-toolbar")
        .test_support()
        .h(rems(2.75))
        .flex_shrink_0()
        .justify_end()
        .px_2()
        .border_b_1()
        .border_color(cx.theme().border)
        .child(actions)
        .into_any_element()
}
pub(super) fn frame(cx: &App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    v_flex()
        .id("sidebar-frame")
        .test_support()
        .size_full()
        .bg(cx.theme().table)
        .border_r_1()
        .border_color(cx.theme().border)
}
pub(super) fn tree_viewport(
    content: impl IntoElement,
) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    div()
        .id("sidebar-tree")
        .test_support()
        .flex_1()
        .min_h_0()
        .p_2()
        .child(content)
}
pub(super) fn navigation(cx: &App) -> gpui_kit::base::ObservedElement<Stateful<Div>> {
    v_flex()
        .id("sidebar-navigation")
        .test_support()
        .flex_shrink_0()
        .p_2()
        .gap_1()
        .border_t_1()
        .border_color(cx.theme().border)
}

/// Retained group navigation tree keyed by database and group identities.
pub struct Sidebar {
    store: Entity<WorkspaceStore>,
    tree: Entity<TreeState>,
    source: Option<(DatabaseId, u64)>,
    database: Option<DatabaseId>,
    selection: Option<GroupId>,
    expanded: BTreeSet<String>,
    _subscriptions: Vec<Subscription>,
}
impl Sidebar {
    /// Build the tree once and reconcile it from observed catalogue changes.
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let tree = cx.new(|cx| TreeState::new(cx));
        let subscriptions = vec![
            cx.observe(&store, |this, _, cx| {
                this.sync(cx);
                cx.notify();
            }),
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
        let mut view = Self {
            store,
            tree,
            source: None,
            database: None,
            selection: None,
            expanded: BTreeSet::new(),
            _subscriptions: subscriptions,
        };
        view.sync(cx);
        view
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
        } else if self.tree.read(cx).selected_index().is_some() {
            self.tree
                .update(cx, |tree, cx| tree.set_selected_index(None, cx));
        }
    }
}
impl Render for Sidebar {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let sidebar = cx.entity();
        let tree = self.tree.clone();
        let store = self.store.clone();
        let menu_store = self.store.clone();
        let context_store = self.store.clone();
        let empty_tree = self.tree.read(cx).entry(0).is_none();
        let drafts = self
            .store
            .read(cx)
            .state()
            .database
            .as_ref()
            .and_then(|db| self.store.read(cx).catalog().read(cx).database(db))
            .map(|db| db.drafts.clone())
            .unwrap_or_default();
        let draft_store = self.store.clone();
        frame(cx)
            .child(toolbar(
                h_flex()
                    .gap_1()
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
                                let history = menu_store.clone();
                                let history_group = parent.clone();
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
                                    PopupMenuItem::new(tr("ui.group_history"))
                                        .disabled(history_group.is_none())
                                        .on_click(move |_, _, cx| {
                                            history.update(cx, |store, cx| {
                                                store.metadata_history(history_group.clone(), cx)
                                            })
                                        }),
                                )
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
                cx,
            ))
            .child(dnd::viewport(
                if empty_tree {
                    v_flex()
                        .p_3()
                        .gap_3()
                        .child(tr("empty_groups"))
                        .child(
                            default_button("empty-add-group")
                                .label(tr("add_group"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    forms::group(this.store.clone(), None, None, window, cx)
                                })),
                        )
                        .into_any_element()
                } else {
                    let root = dnd::root(&store);
                    let content = Tree::new(&self.tree, move |_ix, entry, selected, _, cx| {
                        let group_id = GroupId::new(entry.item().id.as_str());
                        let target = Some(group_id.clone());
                        let store_click = store.clone();
                        let select_tree = tree.clone();
                        let click_sidebar = sidebar.downgrade();
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
                        let row = group_row(
                            SharedString::from(format!(
                                "group-node-{}-{}",
                                state.state().database.as_ref().map_or("", |id| id.as_str()),
                                entry.item().id
                            )),
                            GroupPresentation {
                                label: entry.item().label.clone(),
                                count,
                                depth: entry.depth(),
                                selected,
                            },
                            super::images::stored_icon(
                                group.map_or("folder", |g| g.icon.as_str()),
                                group.and_then(|g| g.icon_blob.as_ref()),
                                cx,
                            ),
                            div()
                                .id(SharedString::from(format!(
                                    "expand-group-{}-{}",
                                    state.state().database.as_ref().map_or("", |id| id.as_str()),
                                    entry.item().id
                                )))
                                .test_support()
                                .w_4()
                                .flex_shrink_0()
                                .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                                .when(entry.is_folder(), |el| {
                                    el.on_click(move |_, _, cx| {
                                        cx.stop_propagation();
                                        toggle_sidebar.update(cx, |sidebar, cx| {
                                            if !sidebar.expanded.remove(&toggle_id) {
                                                sidebar.expanded.insert(toggle_id.clone());
                                            }
                                            sidebar.source = None;
                                            sidebar.sync(cx);
                                            cx.notify();
                                        });
                                    })
                                    .child(icon(
                                        if entry.is_expanded() {
                                            "chevron-down"
                                        } else {
                                            "chevron-right"
                                        },
                                    ))
                                }),
                            cx,
                        );
                        dnd::row(row, &group_id, &store, &sidebar, cx)
                            .on_mouse_down(MouseButton::Left, |_, _, cx| cx.stop_propagation())
                            .on_click(move |_, window, cx| {
                                cx.stop_propagation();
                                if let Some(group) = &target {
                                    store_click.update(cx, |store, cx| {
                                        store.navigate(
                                            Destination::Group(group.clone()),
                                            window,
                                            cx,
                                        )
                                    });
                                    // Project accepted navigation before tree events
                                    // can observe the previous selection.
                                    if let Some(sidebar) = click_sidebar.upgrade() {
                                        sidebar.update(cx, |sidebar, cx| sidebar.sync(cx));
                                    }
                                    select_tree.update(cx, |tree, cx| tree.focus(window, cx));
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
                    .into_any_element();
                    v_flex()
                        .size_full()
                        .child(div().flex_1().min_h_0().child(content))
                        .child(root)
                        .into_any_element()
                },
                &self.store,
            ))
            .child(
                navigation(cx)
                    .child(
                        ListItem::new("ungrouped")
                            .h(rems(2.25))
                            .selected(self.store.read(cx).state().group.is_none())
                            .child(
                                h_flex()
                                    .gap_3()
                                    .child(icon("file"))
                                    .child(tr("ui.ungrouped")),
                            )
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.navigate(Destination::Ungrouped, window, cx)
                                })
                            })),
                    )
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
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.navigate(Destination::Devices, window, cx)
                                });
                            })),
                    )
                    .when(!drafts.is_empty(), |navigation| {
                        navigation.child(
                            v_flex()
                                .id("sidebar-drafts")
                                .max_h(rems(9.))
                                .overflow_y_scroll()
                                .children(drafts.into_iter().enumerate().map(
                                    move |(index, draft)| {
                                        let id = draft.identity.draft;
                                        let store = draft_store.clone();
                                        ListItem::new(SharedString::from(format!(
                                            "draft-{}",
                                            id.as_str()
                                        )))
                                        .h(rems(2.25))
                                        .child(h_flex().gap_3().child(icon("history")).child(
                                            format!("{} {}", tr("ui.local_draft"), index + 1),
                                        ))
                                        .on_click(
                                            move |_, window, cx| {
                                                store.update(cx, |store, cx| {
                                                    store.navigate(
                                                        Destination::Draft(id.clone()),
                                                        window,
                                                        cx,
                                                    )
                                                })
                                            },
                                        )
                                    },
                                )),
                        )
                    }),
            )
    }
}
