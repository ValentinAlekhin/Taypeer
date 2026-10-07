//! Retained causal group/database forms. Widget input stays editable while a captured snapshot saves.
use super::*;
use crate::backend::{Connection, Ticket};
use gpui_kit::component::{
    button::*,
    input::{InputEvent, InputState, Textarea, TextareaState},
    *,
};
use gpui_kit::prelude::FluentBuilder;
use taypeer_core::{DatabaseMetadataPatch, FieldUpdate, GroupMetadataPatch, OperationId};
use taypeer_runtime::Command;
use taypeer_services::{DraftIdentity, DraftSaveOutcome, DraftTarget, MetadataDraftView};

struct Attempt {
    identity: Option<DraftIdentity>,
    input: u64,
    operation: OperationId,
    ticket: Option<Ticket<crate::backend::CapturedSnapshot>>,
}
/// Window-owned metadata input and one explicitly requested immutable publication.
pub(in crate::ui) struct MetadataForm {
    connection: Connection,
    identity: DraftIdentity,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    icon: Option<taypeer_core::IconRef>,
    input: u64,
    durable: Option<u64>,
    patches: Vec<Ticket<DraftIdentity>>,
    attempt: Option<Attempt>,
    error: Option<FormError>,
    closing: bool,
    confirmed: bool,
    failed: bool,
    discard_requested: bool,
    leave_requested: bool,
    _subscriptions: Vec<Subscription>,
}
impl MetadataForm {
    fn new(
        connection: Connection,
        view: MetadataDraftView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let name = input(&view.name, false, window, cx);
        let description = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx);
            state.set_value(view.description.as_deref().unwrap_or_default(), window, cx);
            state
        });
        let subscriptions = vec![
            cx.subscribe(&name, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.patch(Some(input.read(cx).value().to_string()), None, None);
                    cx.notify();
                }
            }),
            cx.subscribe(&description, |this, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    this.patch(None, Some(input.read(cx).value().to_string()), None);
                    cx.notify();
                }
            }),
        ];
        Self {
            connection,
            identity: view.identity,
            name,
            description,
            icon: view.icon,
            input: 0,
            durable: (!view.dirty).then_some(0),
            patches: Vec::new(),
            attempt: None,
            error: None,
            closing: false,
            confirmed: false,
            failed: false,
            discard_requested: false,
            leave_requested: false,
            _subscriptions: subscriptions,
        }
    }
    fn patch(
        &mut self,
        name: Option<String>,
        description: Option<String>,
        icon: Option<taypeer_core::IconRef>,
    ) {
        if !self.connection.control.is_open() {
            return;
        }
        self.input += 1;
        self.error = None;
        if self
            .attempt
            .as_ref()
            .is_some_and(|attempt| attempt.ticket.is_none())
        {
            self.attempt = None;
        }
        let name = name.map_or(FieldUpdate::Keep, FieldUpdate::Set);
        let description = description.map_or(FieldUpdate::Keep, FieldUpdate::Set);
        let command = if matches!(self.identity.target, DraftTarget::Database) {
            Command::PatchDatabaseDraft {
                draft: self.identity.draft.clone(),
                patch: DatabaseMetadataPatch { name, description },
            }
        } else {
            Command::PatchGroupDraft {
                draft: self.identity.draft.clone(),
                patch: GroupMetadataPatch {
                    name,
                    description,
                    icon: icon.map_or(FieldUpdate::Keep, FieldUpdate::Set),
                },
            }
        };
        self.patches.push(self.connection.command(command));
    }
    pub fn request_save(&mut self, cx: &App) {
        if self.saving() || !self.connection.control.is_open() {
            return;
        }
        if let Err(error) = require_name(self.name.read(cx).value().as_str()) {
            self.error = Some(error);
            self.closing = false;
            return;
        }
        if self.attempt.is_none() {
            let Ok(operation) = taypeer_services::new_operation_id() else {
                self.error = Some(FormError::Backend);
                return;
            };
            self.attempt = Some(Attempt {
                identity: None,
                input: self.input,
                operation,
                ticket: None,
            });
        }
        let attempt = self
            .attempt
            .as_mut()
            .expect("requested metadata publication exists");
        attempt.ticket = Some(match &attempt.identity {
            Some(identity) => self
                .connection
                .retry_snapshot(identity.clone(), attempt.operation.clone()),
            None => self
                .connection
                .capture_snapshot(self.identity.draft.clone(), attempt.operation.clone()),
        });
        self.error = None;
        self.closing = true;
    }
    pub fn request_discard(&mut self) {
        self.discard_requested = true;
    }
    pub fn take_discard(&mut self) -> bool {
        std::mem::take(&mut self.discard_requested)
    }
    pub fn request_leave(&mut self) {
        self.leave_requested = true;
    }
    pub fn take_leave(&mut self) -> bool {
        std::mem::take(&mut self.leave_requested)
    }
    pub fn closing(&self) -> bool {
        self.closing
    }
    pub fn stay(&mut self) {
        self.closing = false;
    }
    pub fn target(&self) -> &DraftTarget {
        &self.identity.target
    }
    pub fn draft_id(&self) -> &taypeer_core::DraftId {
        &self.identity.draft
    }
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
    pub fn database(&self) -> &DatabaseId {
        &self.connection.database
    }
    pub fn settled(&self) -> bool {
        self.durable == Some(self.input)
    }
    pub fn dirty(&self) -> bool {
        !self.settled() || self.saving()
    }
    pub fn saving(&self) -> bool {
        self.attempt
            .as_ref()
            .is_some_and(|attempt| attempt.ticket.is_some())
    }
    #[cfg(feature = "ui-test-support")]
    pub fn pending(&self) -> bool {
        !self.patches.is_empty() || self.saving()
    }
    pub fn take_confirmed(&mut self) -> bool {
        std::mem::take(&mut self.confirmed)
    }
    pub fn take_failed(&mut self) -> bool {
        std::mem::take(&mut self.failed)
    }
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        self.patches.retain(|ticket| {
            let Some(result) = ticket.try_take() else {
                return true;
            };
            changed = true;
            match result {
                Ok(identity) => self.identity = identity,
                Err(error) => {
                    self.error = Some(FormError::Runtime(error));
                    self.failed = true;
                    self.closing = false;
                }
            }
            false
        });
        if let Some(result) = self
            .attempt
            .as_ref()
            .and_then(|attempt| attempt.ticket.as_ref())
            .and_then(Ticket::try_take)
        {
            changed = true;
            let attempt = self
                .attempt
                .as_mut()
                .expect("ticket belongs to metadata publication");
            attempt.ticket = None;
            let result = result.and_then(|captured| {
                attempt.identity = Some(captured.identity);
                captured.outcome
            });
            match result {
                Ok(outcome) => {
                    let (identity, operation) = match &outcome {
                        DraftSaveOutcome::Saved {
                            identity,
                            operation,
                        }
                        | DraftSaveOutcome::Unchanged {
                            identity,
                            operation,
                        } => (identity, operation),
                    };
                    if Some(identity) != attempt.identity.as_ref()
                        || operation != &attempt.operation
                    {
                        self.error = Some(FormError::Backend);
                        self.failed = true;
                        self.closing = false;
                    } else {
                        if attempt.input == self.input {
                            self.durable = Some(self.input);
                        }
                        if matches!(outcome, DraftSaveOutcome::Saved { .. }) {
                            self.identity.target = match self.identity.target.clone() {
                                DraftTarget::NewGroup { group, .. } => DraftTarget::Group(group),
                                target => target,
                            };
                        }
                        self.confirmed = true;
                        self.error = None;
                        self.attempt = None;
                    }
                }
                Err(error) => {
                    self.error = Some(FormError::Runtime(error));
                    self.failed = true;
                    self.closing = false;
                    if matches!(
                        error,
                        taypeer_runtime::RuntimeError::Service(
                            taypeer_services::ServiceError::InvalidInput
                        )
                    ) || attempt.input != self.input
                    {
                        self.attempt = None;
                    }
                }
            }
        }
        changed
    }
}
impl Render for MetadataForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let invalid_name =
            self.input > 0 && require_name(self.name.read(cx).value().as_str()).is_err();
        let icon = self.icon.clone().map(|selected| {
            let selected = crate::ui_state::icon_name(&selected);
            icon_choice(default_button("group-icon"), &selected)
                .on_click(cx.listener(|this, _, window, cx| {
                    let form = cx.entity().downgrade();
                    taypeer_ui::icons::choose(
                        this.icon
                            .as_ref()
                            .map(crate::ui_state::icon_name)
                            .unwrap_or_default(),
                        move |name, cx| {
                            if let Ok(key) = taypeer_core::LucideKey::try_from(name.to_owned()) {
                                let _ = form.update(cx, |form, cx| {
                                    let icon = taypeer_core::IconRef::Lucide(key);
                                    form.icon = Some(icon.clone());
                                    form.patch(None, None, Some(icon));
                                    cx.notify();
                                });
                            }
                        },
                        window,
                        cx,
                    );
                }))
                .into_any_element()
        });
        v_flex()
            .key_context("TaypeerMetadata")
            .on_action(cx.listener(|this, _: &crate::actions::SaveEntry, _, cx| {
                this.request_save(cx);
                cx.stop_propagation();
                cx.notify();
            }))
            .on_action(
                cx.listener(|this, _: &crate::actions::CancelEditing, _, cx| {
                    this.request_leave();
                    cx.stop_propagation();
                    cx.notify();
                }),
            )
            .text_sm()
            .child(metadata_fields(
                &self.name,
                &self.description,
                invalid_name,
                icon,
                cx,
            ))
            .when_some(self.error, |el, error| {
                el.child(div().text_color(cx.theme().danger).child(tr(error.key())))
            })
    }
}
fn metadata_fields(
    name: &Entity<InputState>,
    description: &Entity<TextareaState>,
    invalid_name: bool,
    icon: Option<AnyElement>,
    cx: &App,
) -> AnyElement {
    v_flex()
        .child(taypeer_ui::forms::form_row(
            "name",
            Some(name.focus_handle(cx)),
            v_flex()
                .gap_1()
                .child(field(name, "name").when(invalid_name, |input| {
                    input.border_1().border_color(cx.theme().danger)
                }))
                .when(invalid_name, |el| {
                    el.child(
                        div()
                            .id("metadata-name-error")
                            .test_support()
                            .aria_label(tr("ui.required_name"))
                            .text_xs()
                            .text_color(cx.theme().danger)
                            .child(tr("ui.required_name")),
                    )
                }),
            cx,
        ))
        .child(taypeer_ui::forms::form_row(
            "ui.description",
            Some(description.focus_handle(cx)),
            div().id("field-ui.description").test_support().child(
                Textarea::new(description)
                    .aria_label(tr("ui.description"))
                    .h(rems(4.5))
                    .appearance(false)
                    .bordered(false)
                    .bg(cx.theme().title_bar)
                    .rounded(rems(0.25))
                    .px_0()
                    .py_0()
                    .font_weight(FontWeight::NORMAL)
                    .text_size(rems(13. / 16.))
                    .line_height(rems(1.25)),
            ),
            cx,
        ))
        .when_some(icon, |el, icon| {
            el.child(taypeer_ui::forms::form_row("ui.icon", None, icon, cx))
        })
        .into_any_element()
}
fn icon_choice(button: Button, selected: &str) -> Button {
    button
        .w(rems(13.5))
        .px(rems(0.625))
        .icon(icon(selected))
        .accessibility_label(tr("ui.choose_icon"))
        .child(
            div()
                .w_full()
                .text_size(rems(13. / 16.))
                .font_weight(FontWeight::NORMAL)
                .child(tr("ui.choose_icon")),
        )
}

#[cfg(feature = "component-rendering")]
/// Retain the real group metadata controls with public text and inert icon selection.
pub fn metadata_preview(window: &mut Window, cx: &mut App) -> AnyView {
    struct Preview {
        name: Entity<InputState>,
        description: Entity<TextareaState>,
    }
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
            v_flex().px_6().text_sm().child(metadata_fields(
                &self.name,
                &self.description,
                false,
                Some(icon_choice(default_button("group-icon"), "folder").into_any_element()),
                cx,
            ))
        }
    }
    let name = input("Сервисы", false, window, cx);
    let description = cx.new(|cx| {
        let mut description = TextareaState::new(window, cx);
        description.set_value("Сервисы для работы", window, cx);
        description
    });
    cx.new(|_| Preview { name, description }).into()
}
pub(in crate::ui) fn show(
    connection: Connection,
    view: MetadataDraftView,
    window: &mut Window,
    cx: &mut App,
) -> Entity<MetadataForm> {
    let title = match view.identity.target {
        DraftTarget::Database => "ui.database_info",
        DraftTarget::Group(_) => "edit_group",
        _ => "add_group",
    };
    let form = cx.new(|cx| MetadataForm::new(connection, view, window, cx));
    let focus = form.read(cx).name.clone();
    let retained = form.clone();
    window.open_dialog(cx, move |dialog, window, cx| {
        let close = form.clone();
        dialog
            .title(tr(title))
            .width(window.rem_size() * 42.5)
            .p_6()
            .overlay_closable(false)
            .close_button(false)
            .child(form.clone())
            .footer(
                h_flex()
                    .justify_end()
                    .gap_2()
                    .child(
                        default_button("cancel-metadata")
                            .label(tr("cancel"))
                            .on_click({
                                let cancel = form.clone();
                                move |_, _, cx| {
                                    cancel.update(cx, |form, cx| {
                                        form.request_discard();
                                        cx.notify();
                                    })
                                }
                            }),
                    )
                    .child(
                        Button::new("save-metadata")
                            .primary()
                            .label(tr("save"))
                            .disabled(form.read(cx).saving())
                            .on_click(move |_, _, cx| {
                                close.update(cx, |form, cx| {
                                    form.request_save(cx);
                                    cx.notify();
                                })
                            }),
                    ),
            )
            .on_cancel({
                let close = form.clone();
                move |_, _, cx| {
                    close.update(cx, |form, cx| {
                        form.request_leave();
                        cx.notify();
                    });
                    false
                }
            })
    });
    window.defer(cx, move |window, cx| {
        focus.update(cx, |input, cx| input.focus(window, cx))
    });
    retained
}
