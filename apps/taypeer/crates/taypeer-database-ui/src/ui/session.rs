//! Welcome and unlock screens; input belongs to the screen lifetime.

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

/// Database welcome and unlock controls; owns input and pending form state.
pub struct SessionView {
    store: Entity<WorkspaceStore>,
    password: Entity<InputState>,
    database: Option<DatabaseId>,
    secret_epoch: u64,
    error: bool,
    _subscriptions: Vec<Subscription>,
}
impl SessionView {
    #[cfg(feature = "ui-test-support")]
    /// Nonsecret diagnostic state for synthetic UI scenario failures.
    pub fn test_status(&self, cx: &App) -> String {
        format!(
            "password_present={}, input_error={}",
            !self.password.read(cx).value().is_empty(),
            self.error
        )
    }
    /// Create retained unlock controls and observe database lifecycle transitions.
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let password = input("", true, window, cx);
        // The screen can be recreated after a lock. An unrelated later notification
        // must not mistake that existing generation for a new lock and erase input.
        let secret_epoch = store.read(cx).secret_epoch();
        let subscriptions = vec![
            cx.observe_in(&store, window, |this, store, window, cx| {
                let state = store.read(cx);
                let epoch = state.secret_epoch();
                if state.state().is_unlocked() || this.secret_epoch != epoch {
                    this.secret_epoch = epoch;
                    this.password
                        .update(cx, |input, cx| input.set_value("", window, cx));
                    this.error = false;
                }
                cx.notify();
            }),
            cx.subscribe_in(
                &password,
                window,
                |this, _, event: &InputEvent, window, cx| {
                    if matches!(event, InputEvent::PressEnter { .. }) {
                        this.unlock(window, cx);
                    } else {
                        this.error = false;
                        cx.notify();
                    }
                },
            ),
        ];
        Self {
            store,
            password,
            database: None,
            secret_epoch,
            error: false,
            _subscriptions: subscriptions,
        }
    }
    fn unlock(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.password.read(cx).value().is_empty() {
            self.error = true;
            cx.notify();
            return;
        }
        self.store.update(cx, |store, cx| {
            store.unlock(self.password.read(cx).value().to_string(), window, cx)
        });
    }

    fn recent_databases(&self, cx: &Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let recent = &store.local(cx).recent;
        v_flex()
            .w(rems(30.))
            .gap_2()
            .when(!recent.is_empty(), |el| {
                el.child(
                    div()
                        .text_sm()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("ui.recent_databases")),
                )
                .child(
                    v_flex()
                        .id("recent-databases")
                        .max_h(rems(16.))
                        .overflow_y_scroll()
                        .children(recent.iter().map(|item| {
                            let path = item.path.clone();
                            let name = path.file_name().unwrap_or_default().to_string_lossy();
                            Button::new(format!("recent-database-{}", item.database.as_str()))
                                .accessibility_label(path.to_string_lossy().into_owned())
                                .ghost()
                                .w_full()
                                .h_auto()
                                .py_2()
                                .justify_start()
                                .icon(icon("database"))
                                .disabled(store.busy())
                                .child(
                                    v_flex()
                                        .min_w_0()
                                        .items_start()
                                        .child(div().truncate().child(name.into_owned()))
                                        .child(
                                            div()
                                                .text_xs()
                                                .text_color(cx.theme().muted_foreground)
                                                .truncate()
                                                .child(path.to_string_lossy().into_owned()),
                                        ),
                                )
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.store.update(cx, |store, cx| {
                                        store.select_path(path.clone(), window, cx)
                                    });
                                }))
                        })),
                )
            })
    }
}
impl Render for SessionView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let database = store.state().database.clone();
        if database != self.database {
            self.database = database.clone();
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
            .as_ref()
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
                    div().w(rems(24.)).child(super::clipboard::secret_field(
                        Input::new(&self.password)
                            .id("unlock-password")
                            .aria_label(tr("password"))
                            .mask_toggle()
                            .bordered(false)
                            .disabled(store.busy()),
                        &self.password,
                        true,
                    )),
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
                        .disabled(store.busy())
                        .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx))),
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
                            Button::new("welcome-receive")
                                .icon(icon("download"))
                                .label(tr("sync.receive"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    this.store.update(cx, |s, cx| {
                                        s.navigate(
                                            crate::ui_state::Destination::Receive,
                                            window,
                                            cx,
                                        )
                                    })
                                })),
                        )
                        .child(
                            Button::new("welcome-open")
                                .icon(icon("folder-open"))
                                .label(tr("ui.open_file"))
                                .on_click(cx.listener(|this, _, window, cx| {
                                    forms::choose_file(this.store.clone(), window, cx)
                                })),
                        ),
                )
                .child(self.recent_databases(cx))
            })
    }
}
