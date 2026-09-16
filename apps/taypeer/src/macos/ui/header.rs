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
            .h(px(34.))
            .pl_0()
            .border_b_1()
            .border_color(cx.theme().border)
            .child(
                h_flex()
                    .size_full()
                    .relative()
                    .px(px(80.))
                    .justify_center()
                    .child(
                        Button::new("database-switcher")
                            .ghost()
                            .compact()
                            .h(px(28.))
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
                                let receive = menu_store.clone();
                                let devices = menu_store.clone();
                                let share = menu_store.clone();
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
                                    .item(PopupMenuItem::new(tr("sync.receive")).on_click(
                                        move |_, window, cx| {
                                            receive.update(cx, |s, cx| {
                                                s.navigate(Destination::Receive, window, cx)
                                            })
                                        },
                                    ))
                                    .item(
                                        PopupMenuItem::new(tr("sync.devices"))
                                            .disabled(selected.is_none())
                                            .on_click(move |_, window, cx| {
                                                devices.update(cx, |s, cx| {
                                                    s.navigate(Destination::Devices, window, cx)
                                                })
                                            }),
                                    )
                                    .item(
                                        PopupMenuItem::new(tr("ui.share"))
                                            .disabled(!state.state().is_unlocked())
                                            .on_click(move |_, window, cx| {
                                                super::sync::share(share.clone(), window, cx)
                                            }),
                                    )
                            }),
                    )
                    .child(div().absolute().right(px(8.)).top(px(1.)).child(
                        icon_button("settings", "sliders-horizontal", "settings").on_click(
                            cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |store, cx| store.settings(cx))
                            }),
                        ),
                    )),
            )
    }
}
