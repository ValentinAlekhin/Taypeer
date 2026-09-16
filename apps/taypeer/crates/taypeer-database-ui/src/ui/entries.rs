//! Search and the virtualized GPUI Kit table projection.

use super::{style::*, workspace::WorkspaceStore};
use crate::{
    actions::EntryEnter,
    ui_state::{Column as EntryColumn, DatabaseId, Destination, EntryId, GroupId, SearchScope},
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputEvent, InputState},
        menu::{DropdownMenu, PopupMenuItem},
        table::{
            Column as TableColumn, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
        },
        tooltip::Tooltip,
        *,
    },
    *,
};

struct EntryTableRow {
    database: DatabaseId,
    id: EntryId,
    title: String,
    username: String,
    url: String,
    notes: String,
    modified: String,
    location: String,
    icon: String,
    icon_blob: Option<taypeer_core::BlobId>,
    foreground: Option<u32>,
    background: Option<u32>,
}

impl EntryTableRow {
    fn text(&self, column: EntryColumn) -> &str {
        match column {
            EntryColumn::Title => &self.title,
            EntryColumn::Username => &self.username,
            EntryColumn::Url => &self.url,
            EntryColumn::Notes => &self.notes,
            EntryColumn::Modified => &self.modified,
            EntryColumn::Location => &self.location,
        }
    }
}

struct TableSync {
    refresh_columns: bool,
    reset_scroll: bool,
    selected_row: Option<usize>,
}

#[derive(PartialEq, Eq)]
struct RowProjection {
    database: Option<DatabaseId>,
    group: Option<GroupId>,
    query: String,
    scope: SearchScope,
}

struct EntryTableDelegate {
    store: Entity<WorkspaceStore>,
    rows: Vec<EntryTableRow>,
    columns: Vec<EntryColumn>,
    widths: Vec<(EntryColumn, f32)>,
    rem_size: Pixels,
    sort: EntryColumn,
    descending: bool,
    projection: Option<RowProjection>,
}

impl EntryTableDelegate {
    fn new(store: Entity<WorkspaceStore>, cx: &App) -> Self {
        let mut delegate = Self {
            store,
            rows: Vec::new(),
            columns: Vec::new(),
            widths: Vec::new(),
            rem_size: cx.theme().font_size,
            sort: EntryColumn::Title,
            descending: false,
            projection: None,
        };
        delegate.sync(cx);
        delegate
    }

    fn sync(&mut self, cx: &App) -> TableSync {
        let (columns, sort, descending, projection, selected, rows) = {
            let store = self.store.read(cx);
            let state = store.state();
            let catalog = store.catalog().read(cx);
            let projection = RowProjection {
                database: state.database.clone(),
                group: state.group.clone(),
                query: state.query.clone(),
                scope: state.scope,
            };
            let selected = state.database.clone().zip(state.selected.clone());
            let rows = state
                .rows(catalog)
                .into_iter()
                .filter_map(|(database, id)| {
                    let entry = catalog.entry(&database, &id)?;
                    Some(EntryTableRow {
                        database: database.clone(),
                        id,
                        title: entry.content.title.clone(),
                        username: entry.content.username.clone(),
                        url: entry.content.url.clone(),
                        notes: entry.content.notes.replace('\n', " "),
                        modified: stamp(entry.modified),
                        location: catalog.group_path(&database, entry.group.as_ref()),
                        icon: entry.content.icon.clone(),
                        icon_blob: entry.content.icon_blob.clone(),
                        foreground: entry.content.foreground,
                        background: entry.content.background,
                    })
                })
                .collect::<Vec<_>>();
            (
                state.columns.clone(),
                state.sort,
                state.descending,
                projection,
                selected,
                rows,
            )
        };

        let refresh_columns = self.rem_size != cx.theme().font_size
            || self.columns != columns
            || self.sort != sort
            || self.descending != descending;
        self.rem_size = cx.theme().font_size;
        let reset_scroll = self
            .projection
            .as_ref()
            .is_some_and(|current| current != &projection);
        self.columns = columns;
        self.sort = sort;
        self.descending = descending;
        self.projection = Some(projection);
        self.rows = rows;
        let selected_row = selected.and_then(|(database, id)| {
            self.rows
                .iter()
                .position(|row| row.database == database && row.id == id)
        });

        TableSync {
            refresh_columns,
            reset_scroll,
            selected_row,
        }
    }

    fn row_target(&self, row: usize) -> Option<(DatabaseId, EntryId)> {
        self.rows
            .get(row)
            .map(|row| (row.database.clone(), row.id.clone()))
    }

    fn width(&self, column: EntryColumn) -> Pixels {
        self.rem_size
            * self
                .widths
                .iter()
                .find_map(|(candidate, width)| (*candidate == column).then_some(*width))
                .unwrap_or_else(|| default_column_width(column))
    }

    fn set_widths(&mut self, widths: &[Pixels]) {
        for (column, width) in self.columns.iter().copied().zip(widths.iter().copied()) {
            let width = f32::from(width) / f32::from(self.rem_size);
            if let Some((_, stored)) = self
                .widths
                .iter_mut()
                .find(|(candidate, _)| *candidate == column)
            {
                *stored = width;
            } else {
                self.widths.push((column, width));
            }
        }
    }
}

fn default_column_width(column: EntryColumn) -> f32 {
    (match column {
        EntryColumn::Title => 200.,
        EntryColumn::Username => 180.,
        EntryColumn::Url => 240.,
        EntryColumn::Notes => 280.,
        EntryColumn::Modified => 180.,
        EntryColumn::Location => 240.,
    }) / 16.
}

impl TableDelegate for EntryTableDelegate {
    fn columns_count(&self, _: &App) -> usize {
        self.columns.len()
    }

    fn rows_count(&self, _: &App) -> usize {
        self.rows.len()
    }

    fn column(&self, col_ix: usize, _: &App) -> TableColumn {
        let Some(column) = self.columns.get(col_ix).copied() else {
            return TableColumn::new("", "");
        };
        let sort = if column == self.sort {
            if self.descending {
                ColumnSort::Descending
            } else {
                ColumnSort::Ascending
            }
        } else {
            ColumnSort::Default
        };
        TableColumn::new(column.key(), tr(column.key()))
            .sort(sort)
            .width(self.width(column))
            .min_width(self.rem_size * 6.)
            .p_0()
    }

    fn render_th(
        &mut self,
        col_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let label = self
            .columns
            .get(col_ix)
            .map_or_else(String::new, |column| tr(column.key()).to_string());
        h_flex()
            .h_full()
            .flex_1()
            .min_w_0()
            .px_3()
            .child(div().truncate().child(label))
    }

    fn render_tr(
        &mut self,
        row_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) -> Stateful<Div> {
        let Some(row) = self.rows.get(row_ix) else {
            return div().id("missing-entry-row");
        };
        div()
            .id(SharedString::from(format!(
                "entry-row-{}-{}",
                row.database.as_str(),
                row.id.as_str()
            )))
            .relative()
            .when_some(row.background, |element, color| element.bg(rgba(color)))
            .when_some(row.foreground, |element, color| {
                element.text_color(rgba(color))
            })
    }

    fn render_td(
        &mut self,
        row_ix: usize,
        col_ix: usize,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        let Some(row) = self.rows.get(row_ix) else {
            return div().into_any_element();
        };
        let Some(column) = self.columns.get(col_ix).copied() else {
            return div().into_any_element();
        };
        let value = row.text(column).to_owned();

        h_flex()
            .id(SharedString::from(format!(
                "entry-cell-{}-{}-{}",
                row.database.as_str(),
                row.id.as_str(),
                column.key()
            )))
            .test_support()
            .aria_label(value.clone())
            .size_full()
            .min_w_0()
            .gap_2()
            .px_3()
            .border_r_1()
            .border_color(cx.theme().table_row_border)
            .when(column == EntryColumn::Title, |element| {
                element.child(super::images::stored_icon(
                    &row.icon,
                    row.icon_blob.as_ref(),
                    cx,
                ))
            })
            .when(column != EntryColumn::Title, |element| {
                element.text_xs().text_color(cx.theme().muted_foreground)
            })
            .child(div().flex_1().min_w_0().truncate().child(value.clone()))
            .tooltip(move |window, cx| Tooltip::new(value.clone()).build(window, cx))
            .into_any_element()
    }

    fn render_empty(
        &mut self,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) -> impl IntoElement {
        empty("ui.no_results", cx)
    }

    fn perform_sort(
        &mut self,
        col_ix: usize,
        _: ColumnSort,
        _: &mut Window,
        cx: &mut Context<TableState<Self>>,
    ) {
        let Some(column) = self.columns.get(col_ix).copied() else {
            return;
        };
        self.store.update(cx, |store, cx| store.sort_by(column, cx));
    }

    fn move_column(
        &mut self,
        col_ix: usize,
        to_ix: usize,
        _: &mut Window,
        _: &mut Context<TableState<Self>>,
    ) {
        if col_ix == to_ix || col_ix >= self.columns.len() || to_ix >= self.columns.len() {
            return;
        }
        let column = self.columns.remove(col_ix);
        self.columns.insert(to_ix, column);
    }

    fn cell_text(&self, row_ix: usize, col_ix: usize, _: &App) -> String {
        let Some(row) = self.rows.get(row_ix) else {
            return String::new();
        };
        self.columns
            .get(col_ix)
            .map_or_else(String::new, |column| row.text(*column).to_owned())
    }
}

#[derive(Clone, Copy)]
enum ProgrammaticSelection {
    Row(usize),
    Clear,
}

/// Virtualized entry table with retained search, selection, and column geometry.
pub struct Entries {
    store: Entity<WorkspaceStore>,
    search: Entity<InputState>,
    table: Entity<TableState<EntryTableDelegate>>,
    placeholder: SharedString,
    programmatic_selection: Option<ProgrammaticSelection>,
    _subscriptions: Vec<Subscription>,
}

impl Entries {
    /// Create the table and its observation lifetime outside rendering.
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let search = input("", false, window, cx);
        let delegate = EntryTableDelegate::new(store.clone(), cx);
        let table = cx.new(|cx| {
            TableState::new(delegate, window, cx)
                .loop_selection(false)
                .row_selectable(true)
                .col_selectable(false)
                .cell_selectable(false)
                .sortable(true)
                .col_resizable(true)
                .col_movable(true)
        });
        let subscriptions = vec![
            cx.observe_in(&store, window, |this, _, window, cx| {
                this.sync_search(window, cx);
                this.sync_table(cx);
            }),
            cx.observe_global_in::<Theme>(window, |this, window, cx| {
                this.sync_search(window, cx);
                this.sync_table(cx);
            }),
            cx.subscribe_in(&table, window, Self::on_table_event),
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
        let mut this = Self {
            store,
            search,
            table,
            placeholder: "".into(),
            programmatic_selection: None,
            _subscriptions: subscriptions,
        };
        this.sync_search(window, cx);
        this.sync_table(cx);
        this
    }

    /// Focus the retained search input for the window search command.
    pub fn focus_search(&self, window: &mut Window, cx: &mut Context<Self>) {
        self.search.update(cx, |input, cx| input.focus(window, cx));
    }

    fn sync_search(&mut self, window: &mut Window, cx: &mut Context<Self>) {
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
    }

    fn sync_table(&mut self, cx: &mut Context<Self>) {
        let sync = self.table.update(cx, |table, cx| {
            let sync = table.delegate_mut().sync(cx);
            if sync.refresh_columns {
                table.refresh(cx);
            }
            if sync.reset_scroll && table.delegate().rows_count(cx) > 0 {
                table.scroll_to_row(0, cx);
            }
            cx.notify();
            sync
        });
        let current = self.table.read(cx).selected_row();
        let selection = match (current, sync.selected_row) {
            (Some(_), None) => ProgrammaticSelection::Clear,
            (current, Some(row)) if current != Some(row) => ProgrammaticSelection::Row(row),
            _ => return,
        };

        self.programmatic_selection = Some(selection);
        self.table.update(cx, |table, cx| match selection {
            ProgrammaticSelection::Row(row) => table.set_selected_row(row, cx),
            ProgrammaticSelection::Clear => table.clear_selection(cx),
        });
    }

    fn consume_programmatic_selection(&mut self, event: &TableEvent) -> bool {
        let matches = match (self.programmatic_selection, event) {
            (Some(ProgrammaticSelection::Row(expected)), TableEvent::SelectRow(actual)) => {
                expected == *actual
            }
            (Some(ProgrammaticSelection::Clear), TableEvent::ClearSelection) => true,
            _ => false,
        };
        if matches {
            self.programmatic_selection = None;
        }
        matches
    }

    fn on_table_event(
        &mut self,
        table: &Entity<TableState<EntryTableDelegate>>,
        event: &TableEvent,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        match event {
            TableEvent::SelectRow(row) => {
                if self.consume_programmatic_selection(event) {
                    return;
                }
                let target = table.read(cx).delegate().row_target(*row);
                table.focus_handle(cx).focus(window, cx);
                if let Some((database, id)) = target {
                    self.store.update(cx, |store, cx| {
                        store.navigate(Destination::Entry(database, id), window, cx)
                    });
                }
            }
            TableEvent::DoubleClickedRow(row) => {
                if table.read(cx).delegate().row_target(*row).is_some() {
                    self.store.update(cx, |store, cx| store.begin_edit(cx));
                }
            }
            TableEvent::ClearSelection => {
                if self.consume_programmatic_selection(event) {
                    return;
                }
                self.store.update(cx, |store, cx| {
                    store.navigate(Destination::ClearEntry, window, cx)
                });
            }
            TableEvent::ColumnWidthsChanged(widths) => {
                table.update(cx, |table, _| table.delegate_mut().set_widths(widths));
            }
            TableEvent::MoveColumn(from, to) => {
                self.store
                    .update(cx, |store, cx| store.move_column(*from, *to, cx));
            }
            _ => {}
        }
    }
}

impl Render for Entries {
    fn render(&mut self, _window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let state = store.state();
        let scope = state.scope;
        let can_add = state.group.is_some();
        let bookmark = state.bookmark.is_some();
        let row_count = self.table.read(cx).delegate().rows.len();
        let menu_store = self.store.clone();
        let scope_store = self.store.clone();

        v_flex()
            .size_full()
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
                                .id("search")
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
                                for column in EntryColumn::ALL {
                                    let target = menu_store.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(tr(column.key()))
                                            .checked(columns.contains(&column))
                                            .disabled(column == EntryColumn::Title)
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
            .when(bookmark, |element| {
                element.child(
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
            .child(
                div()
                    .flex_1()
                    .min_h_0()
                    .child(DataTable::new(&self.table).bordered(false).stripe(false)),
            )
            .child(
                div()
                    .px_3()
                    .py_2()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(format!("{}: {}", tr("ui.entry_count"), row_count)),
            )
    }
}
