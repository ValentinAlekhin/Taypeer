//! Device and invitation screens; all networking belongs to the background runtime.
use super::{style::*, workspace::WorkspaceStore};
use crate::ui_state::{Destination, InvitationAction, Route, parse_invitation, unix_seconds};
use gpui_kit::{
    component::{
        button::*,
        input::{Input, InputState},
        *,
    },
    prelude::FluentBuilder,
    *,
};
use taypeer_trust::InvitationStatus;

pub(super) struct SyncView {
    store: Entity<WorkspaceStore>,
    code: Entity<InputState>,
    error: Option<&'static str>,
    selecting: bool,
    epoch: u64,
    _subscription: Subscription,
}
impl SyncView {
    pub fn new(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let code = input("", true, window, cx);
        let epoch = store.read(cx).secret_epoch();
        let subscription = cx.observe_in(&store, window, |this, store, window, cx| {
            let epoch = store.read(cx).secret_epoch();
            let route = store.read(cx).state().route;
            if epoch != this.epoch || route != Route::Receive {
                this.code
                    .update(cx, |input, cx| input.set_value("", window, cx));
                this.epoch = epoch;
                this.error = None;
            }
            cx.notify();
        });
        Self {
            store,
            code,
            error: None,
            selecting: false,
            epoch,
            _subscription: subscription,
        }
    }
    fn connect(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.selecting || self.store.read(cx).sync().busy() {
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
        self.error = None;
        self.selecting = true;
        let epoch = self.epoch;
        let prompt = cx.prompt_for_new_path(std::path::Path::new("."), Some("received.taypeer"));
        cx.spawn_in(window, async move |this, cx| {
            let selected = prompt.await;
            let _ = this.update_in(cx, |this, window, cx| {
                this.selecting = false;
                if epoch != this.store.read(cx).secret_epoch()
                    || this.store.read(cx).state().route != Route::Receive
                {
                    return;
                }
                match selected {
                    Ok(Ok(Some(path))) => {
                        this.code
                            .update(cx, |input, cx| input.set_value("", window, cx));
                        this.store
                            .update(cx, |store, cx| store.join_database(code, path, cx));
                    }
                    Ok(Ok(None)) => {}
                    _ => this.error = Some("ui.file_error"),
                }
                cx.notify();
            });
        })
        .detach();
        cx.notify();
    }
    fn receive(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let sync = state.sync();
        v_flex()
            .id("receive-database")
            .size_full()
            .overflow_y_scroll()
            .p_8()
            .gap_5()
            .child(
                Button::new("receive-back")
                    .ghost()
                    .label(tr("ui.back"))
                    .on_click(cx.listener(|this, _, window, cx| {
                        this.store.update(cx, |store, cx| {
                            store.navigate(Destination::Home, window, cx)
                        });
                    })),
            )
            .child(
                div()
                    .text_xl()
                    .font_weight(FontWeight::SEMIBOLD)
                    .child(tr("sync.receive")),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sync.receive_hint")),
            )
            .child(
                div().max_w(px(680.)).child(super::clipboard::secret_field(
                    Input::new(&self.code)
                        .aria_label(tr("sync.code"))
                        .mask_toggle()
                        .disabled(sync.busy() || self.selecting),
                    &self.code,
                    true,
                )),
            )
            .child(
                h_flex()
                    .gap_3()
                    .child(
                        Button::new("connect-invitation")
                            .primary()
                            .label(tr("sync.connect"))
                            .disabled(sync.busy() || self.selecting)
                            .on_click(cx.listener(|this, _, window, cx| this.connect(window, cx))),
                    )
                    .child(
                        Button::new("pause-receive")
                            .label(tr("sync.pause"))
                            .disabled(!sync.busy() && !sync.waiting())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.store.update(cx, |s, cx| s.pause_sync_task(cx))
                            })),
                    ),
            )
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(sync.status)),
            )
            .when_some(self.error.or(sync.error), |el, error| {
                el.child(div().text_color(cx.theme().danger).child(tr(error)))
            })
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
                        Button::new(SharedString::from(format!("resume-{request}")))
                            .label(tr("sync.resume"))
                            .disabled(sync.busy())
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.store.update(cx, |s, cx| s.resume_join(request, cx))
                            })),
                    )
            }))
            .into_any_element()
    }
    fn devices(&self, cx: &mut Context<Self>) -> AnyElement {
        let state = self.store.read(cx);
        let sync = state.sync();
        let database = state.state().database.as_ref();
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
                        Button::new("share-database")
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
            .child(
                h_flex()
                    .py_2()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().w(px(220.)).child(tr("sync.device")))
                    .child(div().w(px(150.)).child(tr("sync.role")))
                    .child(div().flex_1().child(tr("sync.last_exchange"))),
            );
        if let Some(info) = info {
            for device in &info.devices {
                let id = device.id;
                let full = id.to_string();
                let label = if device.local {
                    format!("{} · {}", state.local().device_name, tr("sync.this_device"))
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
                content = content.child(
                    h_flex()
                        .py_3()
                        .gap_2()
                        .border_b_1()
                        .border_color(cx.theme().border)
                        .child(
                            Button::new(SharedString::from(format!("identity-{full}")))
                                .ghost()
                                .w(px(220.))
                                .justify_start()
                                .label(label)
                                .tooltip(full.clone())
                                .on_click(move |_, window, cx| {
                                    let full = full.clone();
                                    window.open_dialog(cx, move |dialog, _, _| {
                                        dialog.title(tr("sync.identity")).child(full.clone())
                                    });
                                }),
                        )
                        .child(div().w(px(140.)).child(tr(if device.manager {
                            "sync.manager"
                        } else {
                            "sync.trusted"
                        })))
                        .child(div().flex_1().text_sm().child(exchange))
                        .when(!device.local, |el| {
                            el.child(
                                Button::new(SharedString::from(format!("exchange-{id}")))
                                    .ghost()
                                    .label(tr("sync.now"))
                                    .disabled(busy)
                                    .on_click(cx.listener(move |this, _, _, cx| {
                                        this.store.update(cx, |s, cx| s.sync_now(Some(id), cx))
                                    })),
                            )
                        }),
                );
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
                                Button::new(SharedString::from(format!("reject-{request}")))
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
                use crate::backend::ApplicationStatus;
                let text = match application {
                    ApplicationStatus::Applied => tr("sync.applied").to_string(),
                    ApplicationStatus::Pending(count) => {
                        format!("{}: {count}", tr("sync.pending_review"))
                    }
                    ApplicationStatus::Failed => tr("sync.apply_failed").to_string(),
                };
                el.child(section("sync.local_application")).child(text)
            })
            .when(!state.state().is_unlocked(), |el| {
                el.child(tr("sync.locked_hint"))
            })
            .child(
                div()
                    .text_sm()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("sync.receipt_hint")),
            )
            .child(
                Button::new("sync-all")
                    .label(tr("sync.now"))
                    .disabled(busy || info.is_none())
                    .on_click(cx.listener(|this, _, _, cx| {
                        this.store.update(cx, |s, cx| s.sync_now(None, cx))
                    })),
            )
            .into_any_element()
    }
}
impl Render for SyncView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        if self.store.read(cx).state().route == Route::Receive {
            self.receive(cx)
        } else {
            self.devices(cx)
        }
    }
}

pub(super) fn share(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut App) {
    let state = store.read(cx);
    let managing = state
        .sync()
        .snapshot
        .databases
        .iter()
        .any(|d| Some(&d.database) == state.state().database.as_ref() && d.managing);
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
    window.open_dialog(cx, move |dialog, _, _| {
        let close = store.clone();
        dialog
            .title(tr("ui.share"))
            .width(px(744.))
            .child(view.clone())
            .on_close(move |_, _, cx| close.update(cx, |s, cx| s.pause_sync_task(cx)))
    });
}
struct InvitationView {
    store: Entity<WorkspaceStore>,
    _subscription: Subscription,
}
impl InvitationView {
    fn copy_code(&self, cx: &App) {
        if let Some(invitation) = &self.store.read(cx).sync().invitation
            && invitation.expires > unix_seconds()
            && let Some(platform) = cx.try_global::<crate::macos::platform::Platform>()
        {
            platform.copy(invitation.code.to_string(), true, true);
        }
    }
}
impl Render for InvitationView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let state = self.store.read(cx);
        let sync = state.sync();
        let mut content = v_flex()
            .gap_4()
            .min_h(px(300.))
            .child(tr("sync.share_hint"))
            .when_some(sync.error, |el, key| {
                el.child(div().text_color(cx.theme().danger).child(tr(key)))
            });
        if let Some(invitation) = &sync.invitation {
            let id = invitation.id;
            let remaining = invitation.expires.saturating_sub(unix_seconds());
            content = content
                .child(
                    h_flex()
                        .gap_6()
                        .when_some(invitation.qr.as_ref(), |el, (width, modules)| {
                            let width = *width;
                            let modules = modules.clone();
                            el.child(
                                canvas(
                                    |_, _, _| (),
                                    move |bounds, _, window, _| {
                                        window.paint_quad(fill(bounds, rgb(0xffffff)));
                                        let unit =
                                            f32::from(bounds.size.width) / (width + 8) as f32;
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
                                .size(px(280.))
                                .flex_shrink_0(),
                            )
                        })
                        .child(
                            v_flex()
                                .gap_4()
                                .flex_1()
                                .child(tr("sync.code_ready"))
                                .when(invitation.qr.is_none(), |el| {
                                    el.child(tr("sync.qr_unavailable"))
                                })
                                .child(
                                    Button::new("copy-invitation")
                                        .label(tr("sync.copy_code"))
                                        .on_click(cx.listener(|this, _, _, cx| this.copy_code(cx))),
                                )
                                .child(format!(
                                    "{} {:02}:{:02}",
                                    tr("sync.expires_in"),
                                    remaining / 60,
                                    remaining % 60
                                )),
                        ),
                )
                .child(
                    Button::new("cancel-invitation")
                        .label(tr("sync.cancel_invitation"))
                        .disabled(sync.busy())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.store.update(cx, |s, cx| {
                                s.invitation_action(id, InvitationAction::Cancel, cx)
                            })
                        })),
                );
            if let Some(database) = sync
                .snapshot
                .databases
                .iter()
                .find(|d| d.database == invitation.database)
                && let Some((request, InvitationStatus::Requested(proof))) = database
                    .invitations
                    .iter()
                    .find(|(request, _)| *request == id)
            {
                let request = *request;
                content = content
                    .child(format!(
                        "{}: {}",
                        tr("sync.requesting_device"),
                        proof.recipient.device
                    ))
                    .child(
                        h_flex()
                            .gap_3()
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
                            )
                            .child(
                                Button::new("reject-invite")
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
                            ),
                    );
            }
        } else {
            content = content.child(tr(sync.status));
        }
        content
    }
}
