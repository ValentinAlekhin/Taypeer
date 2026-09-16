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
    /// Only an empty unlock input is safe for a synthetic layout capture.
    pub fn test_capture_allowed(&self, cx: &App) -> bool {
        self.password.read(cx).value().is_empty()
    }
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
        let database = store.read(cx).state().database.clone();
        if database.is_some() {
            password.update(cx, |input, cx| input.focus(window, cx));
        }
        let subscriptions = vec![
            cx.observe_in(&store, window, |this, store, window, cx| {
                let state = store.read(cx);
                let epoch = state.secret_epoch();
                let database = state.state().database.clone();
                let changed_database = database != this.database;
                if changed_database || state.state().is_unlocked() || this.secret_epoch != epoch {
                    this.database = database.clone();
                    this.secret_epoch = epoch;
                    this.password.update(cx, |input, cx| {
                        input.set_value("", window, cx);
                        if changed_database && database.is_some() {
                            input.focus(window, cx);
                        }
                    });
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
            database,
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
        v_flex().w_full().gap_4().when(!recent.is_empty(), |el| {
            el.child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ui.recent_databases")),
            )
            .child(
                v_flex()
                    .id("recent-databases")
                    .max_h(rems(15.))
                    .overflow_y_scroll()
                    .children(recent.iter().map(|item| {
                        let path = item.path.clone();
                        let name = path
                            .file_name()
                            .unwrap_or_default()
                            .to_string_lossy()
                            .into_owned();
                        div()
                            .w_full()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .child(
                                Button::new(format!("recent-database-{}", item.database.as_str()))
                                    .accessibility_label(path.to_string_lossy().into_owned())
                                    .ghost()
                                    .w_full()
                                    .h(rems(3.75))
                                    .px_2()
                                    .disabled(store.busy())
                                    .child(
                                        h_flex().w_full().gap_4().child(icon("database")).child(
                                            v_flex()
                                                .flex_1()
                                                .min_w_0()
                                                .items_start()
                                                .gap_1()
                                                .child(div().truncate().child(name))
                                                .child(
                                                    div()
                                                        .text_sm()
                                                        .text_color(cx.theme().muted_foreground)
                                                        .truncate()
                                                        .child(display_path(
                                                            path.parent().unwrap_or(&path),
                                                        )),
                                                ),
                                        ),
                                    )
                                    .on_click(cx.listener(move |this, _, window, cx| {
                                        this.store.update(cx, |store, cx| {
                                            store.select_path(path.clone(), window, cx)
                                        });
                                    })),
                            )
                    })),
            )
        })
    }

    fn welcome(&self, cx: &Context<Self>) -> impl IntoElement {
        v_flex()
            .id("welcome-screen")
            .w(rems(27.5))
            .max_w_full()
            .gap_12()
            .child(
                v_flex()
                    .gap_5()
                    .child(
                        div()
                            .text_2xl()
                            .font_weight(FontWeight::MEDIUM)
                            .child("Taypeer"),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(
                                Button::new("welcome-open")
                                    .min_w(rems(9.75))
                                    .icon(icon("folder-open"))
                                    .label(tr("ui.welcome_open"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        forms::choose_file(this.store.clone(), window, cx)
                                    })),
                            )
                            .child(
                                Button::new("welcome-create")
                                    .w(rems(8.125))
                                    .icon(icon("plus"))
                                    .label(tr("ui.welcome_create"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        forms::database(this.store.clone(), None, window, cx)
                                    })),
                            ),
                    ),
            )
            .when(!self.store.read(cx).local(cx).recent.is_empty(), |el| {
                el.child(self.recent_databases(cx))
            })
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("welcome-receive")
                            .min_w(rems(10.5))
                            .label(tr("ui.welcome_receive"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.navigate(
                                        crate::ui_state::Destination::Receive,
                                        window,
                                        cx,
                                    )
                                });
                            })),
                    )
                    .child(
                        Button::new("welcome-import")
                            .w(rems(10.))
                            .label(tr("ui.welcome_import"))
                            .disabled(true)
                            .tooltip(tr("ui.import_unavailable")),
                    ),
            )
    }

    fn unlock_screen(&self, cx: &Context<Self>) -> impl IntoElement {
        let store = self.store.read(cx);
        let database = store
            .state()
            .database
            .as_ref()
            .and_then(|id| store.catalog().read(cx).database(id));
        let path = database
            .map(|db| display_path(&db.path))
            .unwrap_or_default();
        let name = database
            .map(|db| {
                db.path
                    .file_name()
                    .unwrap_or_default()
                    .to_string_lossy()
                    .into_owned()
            })
            .unwrap_or_default();
        v_flex()
            .id("unlock-screen")
            .w(rems(34.))
            .max_w_full()
            .gap_8()
            .child(
                v_flex()
                    .gap_4()
                    .child(
                        h_flex()
                            .gap_3()
                            .child(icon("lock-keyhole"))
                            .child(div().font_weight(FontWeight::SEMIBOLD).child(name)),
                    )
                    .child(
                        div()
                            .id("unlock-path")
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .truncate()
                            .child(path),
                    ),
            )
            .child(
                h_flex()
                    .items_start()
                    .gap_4()
                    .child(
                        div()
                            .w(rems(9.))
                            .flex_shrink_0()
                            .pt_2()
                            .text_sm()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("ui.master_password")),
                    )
                    .child(
                        v_flex()
                            .flex_1()
                            .min_w_0()
                            .gap_2()
                            .child(super::clipboard::secret_field(
                                Input::new(&self.password)
                                    .id("unlock-password")
                                    .aria_label(tr("ui.master_password"))
                                    .mask_toggle()
                                    .bordered(false)
                                    .bg(cx.theme().secondary)
                                    .disabled(store.busy()),
                                &self.password,
                                true,
                            ))
                            .when(self.error, |el| {
                                el.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().danger)
                                        .child(tr("file_empty_password")),
                                )
                            })
                            .when_some(store.notice(), |el, notice| {
                                el.child(
                                    div()
                                        .text_sm()
                                        .text_color(cx.theme().danger)
                                        .child(tr(notice)),
                                )
                            }),
                    ),
            )
            .child(
                h_flex()
                    .mt_2()
                    .gap_3()
                    .child(
                        Button::new("unlock-back")
                            .w(rems(6.5))
                            .label(tr("ui.back"))
                            .disabled(store.busy())
                            .on_click(cx.listener(|this, _, window, cx| {
                                this.store.update(cx, |store, cx| {
                                    store.navigate(
                                        crate::ui_state::Destination::CloseDatabase,
                                        window,
                                        cx,
                                    )
                                });
                            })),
                    )
                    .child(
                        Button::new("unlock-touch-id")
                            .w(rems(8.))
                            .icon(icon("fingerprint"))
                            .label(tr("ui.touch_id"))
                            .disabled(true)
                            .tooltip(tr("ui.touch_id_unavailable")),
                    )
                    .child(div().flex_1())
                    .child(
                        Button::new("unlock")
                            .w(rems(12.5))
                            .custom(
                                ButtonCustomVariant::new(cx)
                                    .color(cx.theme().foreground)
                                    .foreground(cx.theme().background)
                                    .hover(cx.theme().muted_foreground)
                                    .active(cx.theme().muted_foreground),
                            )
                            .when(!store.busy(), |button| button.bg(cx.theme().foreground))
                            .label(tr("unlock"))
                            .disabled(store.busy())
                            .loading(store.busy())
                            .on_click(cx.listener(|this, _, window, cx| this.unlock(window, cx))),
                    ),
            )
    }
}

fn display_path(path: &std::path::Path) -> String {
    if let Some(home) = std::env::var_os("HOME")
        && let Ok(relative) = path.strip_prefix(std::path::Path::new(&home))
    {
        return if relative.as_os_str().is_empty() {
            "~".into()
        } else {
            format!("~/{}", relative.display())
        };
    }
    path.display().to_string()
}

impl Render for SessionView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let database = self.store.read(cx).state().database.clone();
        v_flex()
            .id("session-screen")
            .size_full()
            .overflow_y_scroll()
            .items_center()
            .pt(rems(10.))
            .pb_8()
            .px_8()
            .child(if database.is_some() {
                self.unlock_screen(cx).into_any_element()
            } else {
                self.welcome(cx).into_any_element()
            })
    }
}
