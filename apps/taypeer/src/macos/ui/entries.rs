//! Search and table projection. No independent copy of catalog data is retained.

use super::{style::*, workspace::WorkspaceStore};
use crate::ui_state::Column;
use crate::{
    macos::actions::{EntryDown, EntryEnter, EntryUp},
    ui_state::*,
};
use gpui_kit::component::tooltip::Tooltip;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputEvent, InputState},
        menu::{DropdownMenu, PopupMenuItem},
        table::*,
        *,
    },
    *,
};

pub(super) struct Entries {
    store: Entity<WorkspaceStore>,
    search: Entity<InputState>,
    placeholder: SharedString,
    focus: FocusHandle,
    scroll: ScrollHandle,
    _subscriptions: Vec<Subscription>,
}
impl Entries {
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = input("", false, window, cx);
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(&search, window, {
                let store = store.clone();
                move |_, input, event: &InputEvent, _, cx| {
                    if matches!(event, InputEvent::Change) {
                        let query = input.read(cx).value().to_string();
                        store.update(cx, |store, cx| {
                            store.set_query(query, cx);
                        });
                    }
                }
            }),
        ];
        Self {
            store,
            search,
            placeholder: "".into(),
            focus: cx.focus_handle(),
            scroll: ScrollHandle::new(),
            _subscriptions: subscriptions,
        }
    }
    pub fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.focus(window, cx));
    }
    fn step(&self, down: bool, window: &mut Window, cx: &mut Context<Self>) {
        let store = self.store.read(cx);
        let rows = store.state().rows(store.catalog().read(cx));
        let selected = rows.iter().position(|(db, id)| {
            Some(*db) == store.state().database && Some(*id) == store.state().selected
        });
        let index = selected.map_or(0, |index| {
            if down {
                (index + 1).min(rows.len().saturating_sub(1))
            } else {
                index.saturating_sub(1)
            }
        });
        if let Some((db, id)) = rows.get(index) {
            let target = Destination::Entry(*db, *id);
            self.store
                .update(cx, |store, cx| store.navigate(target, window, cx));
        }
    }
}
impl Render for Entries {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let query = self.store.read(cx).state().query.clone();
        if self.search.read(cx).value().as_str() != query {
            self.search
                .update(cx, |input, cx| input.set_value(query, window, cx));
        }
        let placeholder = tr("ui.search");
        if self.placeholder != placeholder {
            self.placeholder = placeholder.clone();
            self.search.update(cx, |input, cx| {
                input.set_placeholder(placeholder, window, cx)
            });
        }
        let store = self.store.read(cx);
        let state = &store.state();
        let catalog = store.catalog().read(cx);
        let rows = state.rows(catalog);
        let scope = state.scope;
        let can_add = state.group.is_some();
        let bookmark = state.bookmark.is_some();
        let menu_store = self.store.clone();
        let scope_store = self.store.clone();
        let mut heading = TableRow::new().h(rems(2.375));
        for (index, column) in state.columns.iter().copied().enumerate() {
            let selected = state.sort == column;
            let sort_icon = if selected {
                if state.descending {
                    "arrow-down"
                } else {
                    "arrow-up"
                }
            } else {
                "chevrons-up-down"
            };
            let target = self.store.clone();
            heading = heading.child(
                TableHead::new()
                    .p_0()
                    .flex_1()
                    .min_w_0()
                    .border_r_1()
                    .border_color(cx.theme().border)
                    .child(
                        div()
                            .id(("sort", index))
                            .size_full()
                            .px_3()
                            .flex()
                            .items_center()
                            .justify_between()
                            .gap_1()
                            .cursor_pointer()
                            .on_click(move |_, _, cx| {
                                target.update(cx, |store, cx| {
                                    store.sort_by(column, cx);
                                })
                            })
                            .child(div().truncate().child(tr(column.key())))
                            .child(icon(sort_icon)),
                    ),
            );
        }
        let mut body = TableBody::new();
        for (row_index, (db, id)) in rows.iter().copied().enumerate() {
            let Some(entry) = catalog.entry(db, id) else {
                continue;
            };
            let selected = state.database == Some(db) && state.selected == Some(id);
            let mut row = TableRow::new()
                .h(rems(2.25))
                .when_some(entry.content.background, |row, color| row.bg(rgb(color)))
                .when_some(entry.content.foreground, |row, color| {
                    row.text_color(rgb(color))
                })
                .when(selected, |row| row.bg(cx.theme().selection));
            for (col_index, column) in state.columns.iter().enumerate() {
                let text = match column {
                    Column::Title => entry.content.title.clone(),
                    Column::Username => entry.content.username.clone(),
                    Column::Url => entry.content.url.clone(),
                    Column::Notes => entry.content.notes.replace('\n', " "),
                    Column::Modified => stamp(entry.modified),
                    Column::Location => catalog.group_path(db, entry.group),
                };
                let value = text.clone();
                let target = self.store.clone();
                let focus = self.focus.clone();
                row = row.child(
                    TableCell::new().p_0().flex_1().min_w_0().child(
                        div()
                            .id(SharedString::from(format!("entry-{row_index}-{col_index}")))
                            .size_full()
                            .overflow_hidden()
                            .px_3()
                            .flex()
                            .items_center()
                            .gap_2()
                            .cursor_pointer()
                            .when(*column == Column::Title, |el| {
                                el.child(icon(&entry.content.icon))
                            })
                            .when(*column != Column::Title, |el| {
                                el.text_xs().text_color(cx.theme().muted_foreground)
                            })
                            .child(div().flex_1().min_w_0().truncate().child(text))
                            .tooltip(move |window, cx| {
                                Tooltip::new(value.clone()).build(window, cx)
                            })
                            .on_click(move |_, window, cx| {
                                focus.focus(window, cx);
                                target.update(cx, |store, cx| {
                                    store.navigate(Destination::Entry(db, id), window, cx)
                                });
                            }),
                    ),
                );
            }
            body = body.child(row);
        }
        v_flex()
            .size_full()
            .track_focus(&self.focus)
            .key_context("TaypeerEntries")
            .on_action(cx.listener(|this, _: &EntryDown, window, cx| this.step(true, window, cx)))
            .on_action(cx.listener(|this, _: &EntryUp, window, cx| this.step(false, window, cx)))
            .on_action(cx.listener(|this, _: &EntryEnter, _, cx| {
                this.store.update(cx, |store, cx| store.begin_edit(cx))
            }))
            .child(
                h_flex()
                    .h(rems(2.75))
                    .flex_shrink_0()
                    .px_2()
                    .gap_1()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(
                        div().flex_1().min_w_0().child(
                            Input::new(&self.search)
                                .aria_label(tr("ui.search"))
                                .prefix(icon("search"))
                                .cleanable(true)
                                .bordered(false),
                        ),
                    )
                    .child(
                        Button::new("scope")
                            .ghost()
                            .compact()
                            .text_xs()
                            .label(tr(if scope == SearchScope::Current {
                                "ui.this_database"
                            } else {
                                "ui.all_unlocked"
                            }))
                            .child(icon("chevron-down"))
                            .dropdown_menu(move |mut menu, _, _| {
                                for scope in [SearchScope::Current, SearchScope::AllUnlocked] {
                                    let store = scope_store.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(tr(if scope == SearchScope::Current {
                                            "ui.this_database"
                                        } else {
                                            "ui.all_unlocked"
                                        }))
                                        .on_click(
                                            move |_, _, cx| {
                                                store.update(cx, |store, cx| {
                                                    store.set_scope(scope, cx);
                                                })
                                            },
                                        ),
                                    );
                                }
                                menu
                            }),
                    )
                    .child(
                        icon_button("new-entry", "file-plus-2", "new_entry")
                            .disabled(!can_add)
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.navigate(Destination::NewEntry, window, cx)
                                })
                            })),
                    )
                    .child(
                        icon_button("columns", "ellipsis", "ui.columns").dropdown_menu(
                            move |mut menu, _, cx| {
                                let columns = menu_store.read(cx).state().columns.clone();
                                for column in Column::ALL {
                                    let target = menu_store.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(tr(column.key()))
                                            .checked(columns.contains(&column))
                                            .disabled(column == Column::Title)
                                            .on_click(move |_, _, cx| {
                                                target.update(cx, |store, cx| {
                                                    store.toggle_column(column, cx);
                                                })
                                            }),
                                    );
                                }
                                menu
                            },
                        ),
                    ),
            )
            .when(bookmark, |el| {
                el.child(
                    Button::new("back-results")
                        .ghost()
                        .label(tr("ui.back_results"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.store.update(cx, |store, cx| {
                                store.navigate(Destination::SearchResults, window, cx);
                            })
                        })),
                )
            })
            .child(Table::new().child(TableHeader::new().child(heading)))
            .child(
                div()
                    .id("entry-scroll")
                    .flex_1()
                    .min_h_0()
                    .overflow_y_scroll()
                    .track_scroll(&self.scroll)
                    .child(if rows.is_empty() {
                        empty("ui.no_results", cx)
                    } else {
                        Table::new().child(body).into_any_element()
                    }),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{}: {}", tr("ui.entry_count"), rows.len())),
            )
    }
}
