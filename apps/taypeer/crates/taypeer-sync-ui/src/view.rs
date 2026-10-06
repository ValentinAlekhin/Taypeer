//! Device and invitation screens; all networking belongs to the background runtime.
use crate::host::SyncHost;
use crate::{InvitationAction, parse_invitation, unix_seconds};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState, Textarea, TextareaState},
        *,
    },
    prelude::FluentBuilder,
    *,
};
use taypeer_trust::InvitationStatus;
use taypeer_ui::style::*;

/// Network presentation; retains invitation input while the host owns commands.
pub struct SyncView<H: SyncHost> {
    store: Entity<H>,
    code: Entity<TextareaState>,
    password: Entity<InputState>,
    error: Option<&'static str>,
    epoch: u64,
    _subscription: Subscription,
}
impl<H: SyncHost> SyncView<H> {
    /// Create the invitation input and subscriptions once for this view.
    pub fn new(store: Entity<H>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let code = cx.new(|cx| TextareaState::new(window, cx));
        let password = input("", true, window, cx);
        let epoch = store.read(cx).secret_epoch();
        let subscription = cx.observe_in(&store, window, |this, store, window, cx| {
            let epoch = store.read(cx).secret_epoch();
            let receiving = store.read(cx).is_receiving();
            if epoch != this.epoch || !receiving {
                this.code
                    .update(cx, |input, cx| input.set_value("", window, cx));
                this.password
                    .update(cx, |input, cx| input.set_value("", window, cx));
                this.epoch = epoch;
                this.error = None;
            }
            cx.notify();
        });
        Self {
            store,
            code,
            password,
            error: None,
            epoch,
            _subscription: subscription,
        }
    }
    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.store.read(cx).sync(cx).busy() {
            return;
        }
        let code = match parse_invitation(self.code.read(cx).value().as_str()) {
            Ok(code) => code,
            Err(_) => {
                self.error = Some("sync.invalid_code");
                cx.notify();
                return;
            }
        };
        let password = zeroize::Zeroizing::new(self.password.read(cx).value().to_string());
        self.error = None;
        self.code
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.password
            .update(cx, |input, cx| input.set_value("", window, cx));
        self.store.update(cx, |store, cx| {
            store.join_database(code, password.to_string(), cx)
        });
        cx.notify();
    }
    fn receive(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let sync = state.sync(cx);
        v_flex()
            .id("receive-database")
            .test_support()
            .size_full()
            .overflow_y_scroll()
            .items_center()
            .px_6()
            .pt(rems(7.3125))
            .gap_5()
            .child(receive_body(
                &self.code,
                &self.password,
                sync.busy(),
                self.error.or(sync.error),
                receive_actions(
                    default_button("receive-back").on_click(cx.listener(|this, _, window, cx| {
                        this.store.update(cx, |store, cx| store.go_home(window, cx))
                    })),
                    Button::new("connect-invitation")
                        .primary()
                        .disabled(sync.busy())
                        .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
                    (sync.busy() || sync.waiting()).then(|| {
                        default_button("pause-receive")
                            .label(tr("sync.pause"))
                            .disabled(!sync.busy() && !sync.waiting())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| s.pause_sync_task(cx))
                            }))
                    }),
                ),
                cx,
            ))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(sync.status)),
            )
            .when(!sync.joins.is_empty(), |el| {
                el.child(section("sync.pending_connections"))
            })
            .children(sync.joins.iter().map(|(request, pending)| {
                let request = *request;
                h_flex()
                    .gap_3()
                    .py_2()
                    .child(div().flex_1().child(pending.path.display().to_string()))
                    .child(
                        default_button(SharedString::from(format!("resume-{request}")))
                            .label(tr("sync.resume"))
                            .disabled(sync.busy())
                            .on_click(cx.listener(move |this, _, window, cx| {
                                let password = this.password.read(cx).value().to_string();
                                this.password
                                    .update(cx, |input, cx| input.set_value("", window, cx));
                                this.store
                                    .update(cx, |s, cx| s.resume_join(request, password, cx))
                            })),
                    )
            }))
            .into_any_element()
    }
    fn devices(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let sync = state.sync(cx);
        let database = state.selected_database();
        let info = sync
            .snapshot
            .databases
            .iter()
            .find(|d| Some(&d.database) == database);
        let managing = state.writable(cx) && info.is_some_and(|d| d.managing);
        let busy = sync.busy();
        let mut content = v_flex()
            .id("devices-content")
            .size_full()
            .overflow_y_scroll()
            .p_6()
            .gap_4()
            .child(
                h_flex()
                    .justify_between()
                    .child(
                        div()
                            .text_xl()
                            .font_weight(FontWeight::SEMIBOLD)
                            .child(tr("sync.devices")),
                    )
                    .child(
                        default_button("share-database")
                            .label(tr("ui.share"))
                            .disabled(!managing || busy)
                            .tooltip(tr("sync.manager_only"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                share(this.store.clone(), window, cx)
                            })),
                    ),
            )
            .when_some(sync.error, |el, error| {
                el.child(div().text_color(cx.theme().danger).child(tr(error)))
            })
            .child(device_header(cx));
        if let Some(info) = info {
            for (row_index, device) in info.devices.iter().enumerate() {
                let id = device.id;
                let full = id.to_string();
                let label = if device.local {
                    state.device_name(cx).to_owned()
                } else {
                    format!("{} · {}", tr("sync.device"), &full[..12])
                };
                let exchange = device.progress.as_ref().map_or_else(
                    || tr("sync.no_exchange").to_string(),
                    |progress| match progress.result {
                        Ok(report) => format!(
                            "{}: {} · {}: {}",
                            tr("sync.sent"),
                            report.sent,
                            tr("sync.received"),
                            report.received
                        ),
                        Err(_) => tr("sync.network_error").to_string(),
                    },
                );
                let identity = Button::new(SharedString::from(format!("identity-{full}")))
                    .ghost()
                    .tooltip(full.clone())
                    .on_click(move |_, window, cx| {
                        let full = full.clone();
                        window.open_dialog(cx, move |dialog, _, _| {
                            dialog.title(tr("sync.identity")).child(full.clone())
                        });
                    });
                let action = (!device.local).then(|| {
                    Button::new(SharedString::from(format!("exchange-{id}")))
                        .ghost()
                        .disabled(busy)
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.store.update(cx, |s, cx| s.sync_now(Some(id), cx))
                        }))
                });
                content = content.child(device_row(
                    format!("device-row-{id}"),
                    row_index,
                    DeviceCells {
                        identity,
                        identity_label: label,
                        local: device.local,
                        manager: device.manager,
                        exchange,
                        action,
                    },
                    cx,
                ));
            }
            let requests: Vec<_> = info
                .invitations
                .iter()
                .filter(|(_, status)| {
                    matches!(
                        status,
                        InvitationStatus::Available | InvitationStatus::Requested(_)
                    )
                })
                .collect();
            if !requests.is_empty() {
                content = content.child(section("sync.requests"));
            }
            for (request, status) in requests {
                let request = *request;
                let requested = matches!(status, InvitationStatus::Requested(_));
                let label = match status {
                    InvitationStatus::Requested(proof) => format!(
                        "{} · {}",
                        tr("sync.requesting_device"),
                        proof.recipient.device
                    ),
                    _ => format!(
                        "{} · {}",
                        tr("sync.waiting_request"),
                        &request.to_string()[..12]
                    ),
                };
                content = content.child(
                    v_flex().py_3().gap_2().child(label).child(
                        h_flex()
                            .gap_2()
                            .when(requested, |el| {
                                el.child(
                                    Button::new(SharedString::from(format!("approve-{request}")))
                                        .primary()
                                        .label(tr("sync.approve"))
                                        .disabled(!managing || busy)
                                        .on_click(cx.listener(move |this, _, _, cx| {
                                            this.store.update(cx, |s, cx| {
                                                s.invitation_action(
                                                    request,
                                                    InvitationAction::Approve,
                                                    cx,
                                                )
                                            })
                                        })),
                                )
                            })
                            .child(
                                default_button(SharedString::from(format!("reject-{request}")))
                                    .label(tr(if requested {
                                        "sync.reject"
                                    } else {
                                        "sync.cancel_invitation"
                                    }))
                                    .disabled(!managing || busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.invitation_action(
                                                request,
                                                if requested {
                                                    InvitationAction::Reject
                                                } else {
                                                    InvitationAction::Cancel
                                                },
                                                cx,
                                            )
                                        })
                                    })),
                            ),
                    ),
                );
            }
        }
        content
            .child(section("sync.status"))
            .child(tr(sync.database_status(database)))
            .when_some(state.application_status(), |el, application| {
                use taypeer_runtime_client::ApplicationStatus;
                let text = match application {
                    ApplicationStatus::Applied => tr("sync.applied").to_string(),
                    ApplicationStatus::Pending(count) => {
                        format!("{}: {count}", tr("sync.pending_review"))
                    }
                    ApplicationStatus::Failed => tr("sync.apply_failed").to_string(),
                };
                el.child(section("sync.local_application")).child(text)
            })
            .when(!state.is_unlocked(), |el| el.child(tr("sync.locked_hint")))
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sync.receipt_hint")),
            )
            .child(
                default_button("sync-all")
                    .label(tr("sync.now"))
                    .disabled(busy || info.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| s.sync_now(None, cx))
                    })),
            )
            .into_any_element()
    }
}
impl<H: SyncHost> Render for SyncView<H> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.store.read(cx).is_receiving() {
            self.receive(cx)
        } else {
            self.devices(cx)
        }
    }
}

/// Open the invitation dialog and cancel its presentation task on dismissal.
pub fn share<H: SyncHost>(store: Entity<H>, window: &mut Window, cx: &mut App) {
    let state = store.read(cx);
    let managing = state
        .sync(cx)
        .snapshot
        .databases
        .iter()
        .any(|d| Some(&d.database) == state.selected_database() && d.managing);
    if !managing || !state.writable(cx) {
        store.update(cx, |s, cx| s.set_notice("sync.manager_only", cx));
        return;
    }
    store.update(cx, |s, cx| s.share_database(cx));
    let view = cx.new(|cx| {
        let subscription = cx.observe(&store, |_, _, cx| cx.notify());
        InvitationView {
            store: store.clone(),
            _subscription: subscription,
        }
    });
    window.open_dialog(cx, move |dialog, window, _| {
        let close = store.clone();
        dialog
            .title(tr("ui.share"))
            .width(window.rem_size() * 46.5)
            .p_6()
            .child(view.clone())
            .on_close(move |_, _, cx| close.update(cx, |s, cx| s.pause_sync_task(cx)))
    });
}
struct InvitationView<H: SyncHost> {
    store: Entity<H>,
    _subscription: Subscription,
}
impl<H: SyncHost> InvitationView<H> {
    fn copy_code(&self, cx: &App) {
        if let Some(invitation) = &self.store.read(cx).sync(cx).invitation
            && invitation.expires > unix_seconds()
            && let Some(platform) = cx.try_global::<taypeer_desktop_platform::Platform>()
        {
            platform.copy(invitation.code.to_string(), true, true);
        }
    }
}
impl<H: SyncHost> Render for InvitationView<H> {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let sync = state.sync(cx);
        let mut content = v_flex()
            .id("invitation-panel")
            .test_support()
            .gap_6()
            .min_h(rems(18.75))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sync.share_hint")),
            )
            .when_some(sync.error, |el, key| {
                el.child(div().text_color(cx.theme().danger).child(tr(key)))
            });
        if let Some(invitation) = &sync.invitation {
            let id = invitation.id;
            let remaining = invitation.expires.saturating_sub(unix_seconds());
            content = content.child(invitation_layout(
                invitation.code.as_str(),
                invitation.qr.clone(),
                remaining,
                invitation_copy(
                    default_button("copy-invitation")
                        .on_click(cx.listener(|this, _, _, cx| this.copy_code(cx))),
                ),
                cx,
            ));
            let request = sync
                .snapshot
                .databases
                .iter()
                .find(|d| d.database == invitation.database)
                .and_then(|database| {
                    database
                        .invitations
                        .iter()
                        .find(|(request, _)| *request == id)
                })
                .and_then(|(request, status)| match status {
                    InvitationStatus::Requested(proof) => Some((*request, proof.recipient.device)),
                    _ => None,
                });
            if let Some((request, recipient)) = request {
                content = content
                    .child(
                        div()
                            .border_t_1()
                            .border_color(cx.theme().border)
                            .pt_6()
                            .child(format!("{}: {recipient}", tr("sync.requesting_device"))),
                    )
                    .child(
                        div()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr("sync.receive_hint")),
                    )
                    .child(
                        h_flex()
                            .justify_end()
                            .gap_3()
                            .child(
                                default_button("reject-invite")
                                    .label(tr("sync.reject"))
                                    .disabled(sync.busy())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.invitation_action(
                                                request,
                                                InvitationAction::Reject,
                                                cx,
                                            )
                                        })
                                    })),
                            )
                            .child(
                                Button::new("approve-invite")
                                    .primary()
                                    .label(tr("sync.approve"))
                                    .disabled(sync.busy())
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.store.update(cx, |s, cx| {
                                            s.invitation_action(
                                                request,
                                                InvitationAction::Approve,
                                                cx,
                                            )
                                        })
                                    })),
                            ),
                    );
            } else {
                content = content.child(
                    h_flex().justify_end().child(
                        default_button("cancel-invitation")
                            .label(tr("sync.cancel_invitation"))
                            .disabled(sync.busy())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.store.update(cx, |s, cx| {
                                    s.invitation_action(id, InvitationAction::Cancel, cx)
                                })
                            })),
                    ),
                );
            }
        } else {
            content = content.child(tr(sync.status));
        }
        content
    }
}

fn device_cell(label: &str, width: f32, cx: &App) -> AnyElement {
    div()
        .id(SharedString::from(format!("devices-column-{label}")))
        .test_support()
        .w(rems(width))
        .h_full()
        .flex()
        .items_center()
        .px_4()
        .text_size(rems(13. / 16.))
        .text_color(cx.theme().muted_foreground)
        .child(tr(label))
        .into_any_element()
}
fn device_header(cx: &App) -> AnyElement {
    h_flex()
        .id("devices-header")
        .test_support()
        .h(rems(2.375))
        .border_b_1()
        .border_color(cx.theme().border)
        .bg(cx.theme().background)
        .font_weight(FontWeight::NORMAL)
        .text_size(rems(13. / 16.))
        .line_height(rems(1.25))
        .child(device_cell("sync.device", 17.5, cx))
        .child(device_cell("sync.availability", 12.5, cx))
        .child(device_cell("sync.role", 13.75, cx))
        .child(
            div()
                .flex_1()
                .h_full()
                .flex()
                .items_center()
                .px_4()
                .text_color(cx.theme().muted_foreground)
                .child(tr("sync.exchange")),
        )
        .into_any_element()
}
struct DeviceCells {
    identity: Button,
    identity_label: String,
    local: bool,
    manager: bool,
    exchange: String,
    action: Option<Button>,
}
fn device_row(
    row_id: impl Into<SharedString>,
    row_index: usize,
    cells: DeviceCells,
    cx: &App,
) -> AnyElement {
    let DeviceCells {
        identity,
        identity_label,
        local,
        manager,
        exchange,
        action,
    } = cells;
    h_flex()
        .id(row_id.into())
        .test_support()
        .h(rems(3.))
        .border_b_1()
        .border_color(cx.theme().border)
        .when(!row_index.is_multiple_of(2), |row| {
            row.bg(cx.theme().button)
        })
        .text_size(rems(13. / 16.))
        .line_height(rems(1.25))
        .font_weight(FontWeight::NORMAL)
        .text_color(cx.theme().muted_foreground)
        .child(
            div().w(rems(17.5)).px_4().child(
                identity
                    .w_full()
                    .px_0()
                    .text_color(cx.theme().foreground)
                    .accessibility_label(identity_label.clone())
                    .child(
                        div()
                            .w_full()
                            .text_size(rems(13. / 16.))
                            .child(identity_label),
                    ),
            ),
        )
        .child(
            div()
                .w(rems(12.5))
                .h_full()
                .flex()
                .items_center()
                .px_4()
                .child(tr(if local {
                    "sync.this_device"
                } else {
                    "sync.presence_unknown"
                })),
        )
        .child(
            div()
                .w(rems(13.75))
                .h_full()
                .flex()
                .items_center()
                .px_4()
                .child(tr(if manager {
                    "sync.manager"
                } else {
                    "sync.trusted"
                })),
        )
        .child(
            h_flex()
                .flex_1()
                .min_w_0()
                .h_full()
                .pl_4()
                .pr_3()
                .child(div().flex_1().min_w_0().child(exchange))
                .when_some(action, |cell, action| {
                    cell.child(
                        action
                            .w(rems(9.25))
                            .px_0()
                            .text_color(cx.theme().foreground)
                            .accessibility_label(tr("sync.now"))
                            .child(div().text_size(rems(13. / 16.)).child(tr("sync.now"))),
                    )
                }),
        )
        .into_any_element()
}
fn receive_inputs(
    code: &Entity<TextareaState>,
    password: &Entity<InputState>,
    busy: bool,
    error: Option<&'static str>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .id("receive-inputs")
        .test_support()
        .max_w(rems(40.))
        .text_size(rems(13. / 16.))
        .line_height(rems(1.25))
        .font_weight(FontWeight::NORMAL)
        .child(
            div()
                .h(rems(1.5))
                .text_size(rems(1.))
                .font_weight(FontWeight::MEDIUM)
                .child(tr("sync.receive_title")),
        )
        .child(
            div()
                .mt_6()
                .h(rems(1.5))
                .text_color(cx.theme().muted_foreground)
                .child(tr("sync.invitation")),
        )
        .child(
            div().mt_4().child(taypeer_ui::forms::form_row(
                "sync.code",
                Some(code.focus_handle(cx)),
                taypeer_ui::clipboard::secret_textarea(
                    div().id("invitation-code").test_support().child(
                        Textarea::new(code)
                            .aria_label(tr("sync.code"))
                            .appearance(false)
                            .bordered(false)
                            .bg(cx.theme().title_bar)
                            .rounded(rems(0.25))
                            .px_0()
                            .py_0()
                            .font_weight(FontWeight::NORMAL)
                            .text_size(rems(13. / 16.))
                            .line_height(rems(1.25))
                            .h(rems(6.75))
                            .disabled(busy),
                    ),
                    code,
                    true,
                ),
                cx,
            )),
        )
        .when_some(error, |el, key| {
            el.child(
                div()
                    .id("receive-error")
                    .test_support()
                    .aria_label(tr(key))
                    .text_color(cx.theme().danger)
                    .child(tr(key)),
            )
        })
        .child(
            div().mt_5().child(
                taypeer_ui::forms::form_row(
                    "ui.master_password",
                    Some(password.focus_handle(cx)),
                    taypeer_ui::clipboard::secret_field(
                        Input::new(password)
                            .id("invitation-password")
                            .aria_label(tr("ui.master_password"))
                            .appearance(false)
                            .bordered(false)
                            .bg(cx.theme().title_bar)
                            .rounded(rems(0.25))
                            .px(rems(0.625))
                            .py(rems(0.375))
                            .h(rems(2.))
                            .text_size(rems(13. / 16.))
                            .font_weight(FontWeight::NORMAL)
                            .mask_toggle()
                            .disabled(busy),
                        password,
                        true,
                    ),
                    cx,
                )
                .min_h(rems(2.)),
            ),
        )
        .child(
            div()
                .ml(rems(10.))
                .mt_5()
                .min_h(rems(3.75))
                .text_color(cx.theme().muted_foreground)
                .child(tr("sync.receive_hint")),
        )
        .into_any_element()
}
fn receive_actions(back: Button, connect: Button, pause: Option<Button>) -> AnyElement {
    h_flex()
        .justify_between()
        .child(
            back.w(rems(8.))
                .px(rems(0.625))
                .accessibility_label(tr("ui.back"))
                .child(
                    div()
                        .w_full()
                        .text_size(rems(13. / 16.))
                        .child(tr("ui.back")),
                ),
        )
        .child(
            h_flex()
                .gap_3()
                .when_some(pause, |actions, pause| {
                    actions.child(pause.text_size(rems(13. / 16.)))
                })
                .child(
                    connect
                        .w(rems(12.5))
                        .px(rems(0.625))
                        .accessibility_label(tr("sync.connect"))
                        .child(
                            div()
                                .w_full()
                                .text_size(rems(13. / 16.))
                                .child(tr("sync.connect")),
                        ),
                ),
        )
        .into_any_element()
}
fn receive_body(
    code: &Entity<TextareaState>,
    password: &Entity<InputState>,
    busy: bool,
    error: Option<&'static str>,
    actions: AnyElement,
    cx: &App,
) -> AnyElement {
    v_flex()
        .w(rems(40.))
        .max_w_full()
        .font_weight(FontWeight::NORMAL)
        .child(receive_inputs(code, password, busy, error, cx))
        .child(div().mt_10().child(actions))
        .into_any_element()
}
fn invitation_layout(
    code: &str,
    qr: Option<(usize, zeroize::Zeroizing<Vec<u8>>)>,
    remaining: u64,
    copy: AnyElement,
    cx: &App,
) -> AnyElement {
    let mut lines = Vec::new();
    let mut line = String::new();
    for character in code.chars() {
        line.push(character);
        if line.chars().count() == 24 {
            lines.push(std::mem::take(&mut line));
        }
    }
    if !line.is_empty() {
        lines.push(line);
    }
    h_flex()
        .id("invitation-content")
        .test_support()
        .items_start()
        .font_weight(FontWeight::NORMAL)
        .gap_8()
        .when_some(qr.as_ref(), |el, (width, modules)| {
            let width = *width;
            let modules = modules.clone();
            el.child(
                div()
                    .id("invitation-qr")
                    .test_support()
                    .size(rems(14.5))
                    .flex_shrink_0()
                    .child(
                        canvas(
                            |_, _, _| (),
                            move |bounds, _, window, _| {
                                window.paint_quad(fill(bounds, rgb(0xffffff)));
                                let unit = f32::from(bounds.size.width) / (width + 8) as f32;
                                for y in 0..width {
                                    for x in 0..width {
                                        if modules[y * width + x] != 0 {
                                            window.paint_quad(fill(
                                                Bounds::new(
                                                    bounds.origin
                                                        + point(
                                                            px((x + 4) as f32 * unit),
                                                            px((y + 4) as f32 * unit),
                                                        ),
                                                    size(px(unit), px(unit)),
                                                ),
                                                rgb(0),
                                            ));
                                        }
                                    }
                                }
                            },
                        )
                        .size_full(),
                    ),
            )
        })
        .child(
            v_flex()
                .flex_1()
                .min_w_0()
                .gap_4()
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("sync.invitation_code")),
                )
                .child(
                    v_flex()
                        .id("invitation-code-display")
                        .test_support()
                        .h(rems(6.5))
                        .overflow_y_scroll()
                        .font_family(cx.theme().mono_font_family.clone())
                        .text_sm()
                        .line_height(rems(1.25))
                        .children(lines.into_iter().map(|line| div().child(line))),
                )
                .when(qr.is_none(), |el| el.child(tr("sync.qr_unavailable")))
                .child(copy)
                .child(
                    div()
                        .text_xs()
                        .text_color(cx.theme().muted_foreground)
                        .child(format!(
                            "{} {:02}:{:02}",
                            tr("sync.expires_in"),
                            remaining / 60,
                            remaining % 60
                        )),
                ),
        )
        .into_any_element()
}

fn invitation_copy(button: Button) -> AnyElement {
    button
        .w(rems(13.5))
        .px(rems(0.625))
        .icon(icon("copy"))
        .accessibility_label(tr("sync.copy_code"))
        .child(
            div()
                .w_full()
                .text_size(rems(13. / 16.))
                .font_weight(FontWeight::NORMAL)
                .child(tr("sync.copy_code")),
        )
        .into_any_element()
}

#[cfg(feature = "component-rendering")]
#[derive(Clone, Copy)]
/// Public offline layouts without a network or real invitation capability.
pub enum SyncPreviewCase {
    /// Device rows use only the current public projection: unknown remote presence.
    Devices,
    /// Static nonusable invitation code and matching bundled QR.
    Invitation,
    /// Retained receive fields with no submission callback.
    Receive,
}
#[cfg(feature = "component-rendering")]
/// Mount product builders with public design data and inert actions.
pub fn sync_preview(case: SyncPreviewCase, window: &mut Window, cx: &mut App) -> AnyView {
    struct Preview {
        case: SyncPreviewCase,
        code: Entity<TextareaState>,
        password: Entity<InputState>,
        fixture_code: String,
        qr: (usize, zeroize::Zeroizing<Vec<u8>>),
    }
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            match self.case {
                SyncPreviewCase::Devices => v_flex()
                    .id("devices-table")
                    .test_support()
                    .child(device_header(cx))
                    .child(device_row(
                        "device-row-public-local",
                        0,
                        DeviceCells {
                            identity: Button::new("identity-public-local").ghost(),
                            identity_label: "MacBook Pro".to_owned(),
                            local: true,
                            manager: true,
                            exchange: tr("sync.no_exchange").to_string(),
                            action: None,
                        },
                        cx,
                    ))
                    .children(
                        ["000000000001", "000000000002"]
                            .into_iter()
                            .enumerate()
                            .map(|(index, id)| {
                                device_row(
                                    format!("device-row-public-remote-{index}"),
                                    index + 1,
                                    DeviceCells {
                                        identity: Button::new(("identity-public-remote", index))
                                            .ghost(),
                                        identity_label: format!("{} · {id}", tr("sync.device")),
                                        local: false,
                                        manager: false,
                                        exchange: tr("sync.no_exchange").to_string(),
                                        action: Some(
                                            Button::new(("exchange-public", index)).ghost(),
                                        ),
                                    },
                                    cx,
                                )
                            }),
                    )
                    .into_any_element(),
                SyncPreviewCase::Receive => receive_body(
                    &self.code,
                    &self.password,
                    false,
                    None,
                    receive_actions(
                        default_button("receive-back"),
                        Button::new("connect-invitation").primary(),
                        None,
                    ),
                    cx,
                ),
                SyncPreviewCase::Invitation => invitation_layout(
                    &self.fixture_code,
                    Some(self.qr.clone()),
                    272,
                    invitation_copy(default_button("copy-invitation")),
                    cx,
                ),
            }
        }
    }
    let fixture: serde_json::Value = serde_json::from_str(include_str!(
        "../../../../../wireframes/assets/demo-invitation.json"
    ))
    .expect("bundled public invitation fixture");
    let fixture_code = fixture["code"]
        .as_str()
        .expect("public fixture code")
        .to_owned();
    let rows = fixture["modules"]
        .as_array()
        .expect("public fixture QR rows");
    let width = rows.len();
    let modules = rows
        .iter()
        .flat_map(|row| {
            row.as_str()
                .expect("public fixture QR row")
                .bytes()
                .map(|byte| u8::from(byte == b'1'))
        })
        .collect();
    let code = cx.new(|cx| {
        let mut state = TextareaState::new(window, cx);
        state.set_value(fixture_code.clone(), window, cx);
        state
    });
    let password = input("", true, window, cx);
    cx.new(|_| Preview {
        case,
        code,
        password,
        fixture_code,
        qr: (width, zeroize::Zeroizing::new(modules)),
    })
    .into()
}
