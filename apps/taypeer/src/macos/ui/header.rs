//! Window chrome and the database switcher.

use super::{forms, style::*, workspace::WorkspaceStore};
use crate::ui_state::Destination;
use gpui_kit::{
    component::{
        button::*,
        menu::{DropdownMenu, PopupMenuItem},
        *,
    },
    *,
};

pub(super) struct Header {
    store: Entity<WorkspaceStore>,
    _subscription: Subscription,
}
impl Header {
    pub fn new(store: Entity<WorkspaceStore>, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe(&store, |_, _, cx| cx.notify());
        Self {
            store,
            _subscription: subscription,
        }
    }
}
impl Render for Header {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let selected = state.state().database.clone();
        let catalog = state.catalog().clone();
        let title = selected
            .as_ref()
            .and_then(|id| catalog.read(cx).database(id))
            .map(|db| db.name.clone())
            .unwrap_or_else(|| "Taypeer".into());
        let menu_store = self.store.clone();
        TitleBar::new()
            .h(rems(2.75))
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .size_full()
                    .pl_20()
                    .pr_2()
                    .child(
                        div()
                            .text_xs()
                            .text_color(cx.theme().muted_foreground)
                            .child("Taypeer"),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("database-switcher")
                            .ghost()
                            .icon(icon("file-key-2"))
                            .label(title)
                            .child(icon("chevron-down"))
                            .dropdown_menu(move |mut menu, _, cx| {
                                let state = menu_store.read(cx);
                                for id in &state.state().opened {
                                    let Some(db) = catalog.read(cx).database(id) else {
                                        continue;
                                    };
                                    let target = id.clone();
                                    let store = menu_store.clone();
                                    menu = menu.item(
                                        PopupMenuItem::new(db.name.clone())
                                            .icon(icon(if state.state().unlocked.contains(id) {
                                                "file-key-2"
                                            } else {
                                                "lock"
                                            }))
                                            .checked(selected == Some(id.clone()))
                                            .on_click(move |_, window, cx| {
                                                store.update(cx, |store, cx| {
                                                    store.navigate(
                                                        Destination::Database(target.clone()),
                                                        window,
                                                        cx,
                                                    )
                                                })
                                            }),
                                    );
                                }
                                let open = menu_store.clone();
                                let create = menu_store.clone();
                                let lock = menu_store.clone();
                                let close = menu_store.clone();
                                menu.separator()
                                    .item(PopupMenuItem::new(tr("create_db")).on_click(
                                        move |_, window, cx| {
                                            create.update(cx, |store, cx| {
                                                store.navigate(
                                                    Destination::CreateDatabase,
                                                    window,
                                                    cx,
                                                )
                                            })
                                        },
                                    ))
                                    .item(PopupMenuItem::new(tr("ui.open_file")).on_click(
                                        move |_, window, cx| {
                                            forms::choose_file(open.clone(), window, cx)
                                        },
                                    ))
                                    .separator()
                                    .item(
                                        PopupMenuItem::new(tr("lock"))
                                            .disabled(!state.state().is_unlocked())
                                            .on_click(move |_, window, cx| {
                                                lock.update(cx, |store, cx| store.lock(window, cx))
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new(tr("ui.close_database"))
                                            .disabled(selected.is_none())
                                            .on_click(move |_, window, cx| {
                                                close.update(cx, |store, cx| {
                                                    store.navigate(
                                                        Destination::CloseDatabase,
                                                        window,
                                                        cx,
                                                    )
                                                })
                                            }),
                                    )
                                    .separator()
                                    .item(PopupMenuItem::new(tr("ui.share")).disabled(true))
                            }),
                    )
                    .child(div().flex_1())
                    .child(
                        icon_button("settings", "sliders-horizontal", "settings").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |store, cx| store.settings(cx))
                            }),
                        ),
                    ),
            )
    }
}
