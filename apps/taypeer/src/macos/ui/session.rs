//! Welcome and synthetic unlock screens; their input dies when the screen closes.

use super::{forms, style::*, workspace::WorkspaceStore};
use crate::ui_state::DatabaseId;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputEvent, InputState},
        *,
    },
    *,
};

pub(super) struct SessionView {
    store: Entity<WorkspaceStore>,
    password: Entity<InputState>,
    database: Option<DatabaseId>,
    error: bool,
    _subscriptions: Vec<Subscription>,
}
impl SessionView {
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = input("", true, window, cx);
        let subscriptions = vec![
            cx.observe(&store, |_, _, cx| cx.notify()),
            cx.subscribe_in(&password, window, |this, _, event: &InputEvent, _, cx| {
                if matches!(event, InputEvent::PressEnter { .. }) {
                    this.unlock(cx);
                } else {
                    this.error = false;
                    cx.notify();
                }
            }),
        ];
        Self {
            store,
            password,
            database: None,
            error: false,
            _subscriptions: subscriptions,
        }
    }
    fn unlock(&mut self, cx: &mut Context<Self>) {
        if self.password.read(cx).value().is_empty() {
            self.error = true;
            cx.notify();
            return;
        }
        self.store.update(cx, |store, cx| store.unlock(cx));
    }
}
impl Render for SessionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let database = store.state().database;
        if database != self.database {
            self.database = database;
            self.error = false;
            self.password.update(cx, |input, cx| {
                input.set_value("", window, cx);
                if database.is_some() {
                    input.focus(window, cx);
                }
            });
        }
        let store = self.store.read(cx);
        let name = database
            .and_then(|db| store.catalog().read(cx).database(db))
            .map(|db| db.name.clone());
        v_flex()
            .size_full()
            .items_center()
            .justify_center()
            .gap_5()
            .p_8()
            .child(
                icon(if database.is_some() {
                    "lock"
                } else {
                    "file-key-2"
                })
                .size(rems(2.5)),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(name.unwrap_or_else(|| tr("welcome").to_string())),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(if database.is_some() {
                        "ui.unlock_hint"
                    } else {
                        "ui.welcome_hint"
                    })),
            )
            .when(database.is_some(), |el| {
                el.child(
                    div().w(rems(24.)).child(
                        Input::new(&self.password)
                            .aria_label(tr("password"))
                            .mask_toggle()
                            .bordered(false),
                    ),
                )
                .when(self.error, |el| {
                    el.child(
                        div()
                            .text_color(cx.theme().danger)
                            .child(tr("file_empty_password")),
                    )
                })
                .child(
                    Button::new("unlock")
                        .primary()
                        .label(tr("unlock"))
                        .on_click(cx.listener(|this, _, _, cx| this.unlock(cx))),
                )
            })
            .when(database.is_none(), |el| {
                el.child(
                    h_flex()
                        .gap_3()
                        .child(
                            Button::new("welcome-create")
                                .icon(icon("file-plus-2"))
                                .label(tr("create_db"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    forms::database(this.store.clone(), None, window, cx)
                                })),
                        )
                        .child(
                            Button::new("welcome-open")
                                .icon(icon("folder-open"))
                                .label(tr("ui.open_sample"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    forms::choose_sample(this.store.clone(), window, cx)
                                })),
                        ),
                )
            })
    }
}
