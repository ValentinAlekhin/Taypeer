//! Retained causal group/database forms. Widget input stays editable while a captured snapshot saves.
use super::*;
use crate::backend::{Connection, Ticket};
use gpui_kit::component::{
    button::*,
    input::{InputEvent, InputState, Textarea, TextareaState},
    *,
};
use gpui_kit::prelude::FluentBuilder;
use std::time::{Duration, Instant};
use taypeer_core::{DatabaseMetadataPatch, FieldUpdate, GroupMetadataPatch, OperationId};
use taypeer_runtime::Command;
use taypeer_services::{DraftIdentity, DraftSaveOutcome, DraftTarget, MetadataDraftView};

struct Attempt {
    identity: DraftIdentity,
    input: u64,
    operation: OperationId,
    ticket: Option<Ticket<DraftSaveOutcome>>,
    retry_at: Instant,
}
/// Window-owned metadata inputs and the immutable publication they are awaiting.
pub(in crate::ui) struct MetadataForm {
    connection: Connection,
    identity: DraftIdentity,
    name: Entity<InputState>,
    description: Entity<TextareaState>,
    icon: Option<taypeer_core::IconRef>,
    input: u64,
    durable: Option<u64>,
    changed_at: Instant,
    patches: Vec<Ticket<DraftIdentity>>,
    attempt: Option<Attempt>,
    fallback: Option<(u64, Ticket<()>)>,
    error: Option<FormError>,
    patch_error: bool,
    closing: bool,
    confirmed: bool,
    local_only: bool,
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
            changed_at: Instant::now(),
            patches: Vec::new(),
            attempt: None,
            fallback: None,
            error: None,
            patch_error: false,
            closing: false,
            confirmed: false,
            local_only: false,
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
        self.changed_at = Instant::now();
        self.error = None;
        self.patch_error = false;
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
    pub fn request_close(&mut self) {
        self.closing = true;
    }
    pub fn closing(&self) -> bool {
        self.closing
    }
    pub fn target(&self) -> &DraftTarget {
        &self.identity.target
    }
    pub fn database(&self) -> &DatabaseId {
        &self.connection.database
    }
    pub fn settled(&self) -> bool {
        self.durable == Some(self.input) && self.patches.is_empty() && self.fallback.is_none()
    }
    pub fn pending(&self) -> bool {
        !self.settled() && self.error.is_none()
    }
    pub fn take_confirmed(&mut self) -> bool {
        std::mem::take(&mut self.confirmed)
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
                    self.patch_error = true;
                    self.closing = false;
                }
            }
            false
        });
        if let Some((input, ticket)) = &self.fallback
            && let Some(result) = ticket.try_take()
        {
            changed = true;
            match result {
                Ok(()) => {
                    if *input == self.input {
                        self.durable = Some(*input);
                    }
                }
                Err(error) => {
                    self.error = Some(FormError::Runtime(error));
                    self.closing = false;
                }
            }
            self.fallback = None;
        }
        if let Some(result) = self
            .attempt
            .as_ref()
            .and_then(|a| a.ticket.as_ref())
            .and_then(Ticket::try_take)
        {
            changed = true;
            let attempt = self
                .attempt
                .as_mut()
                .expect("captured ticket belongs to its attempt");
            attempt.ticket = None;
            match result {
                Ok(outcome) => {
                    let identity = match &outcome {
                        DraftSaveOutcome::Saved { identity, .. }
                        | DraftSaveOutcome::LocalDraftSaved { identity, .. }
                        | DraftSaveOutcome::Unchanged { identity, .. } => identity,
                    };
                    let operation = match &outcome {
                        DraftSaveOutcome::Saved { operation, .. }
                        | DraftSaveOutcome::LocalDraftSaved { operation, .. }
                        | DraftSaveOutcome::Unchanged { operation, .. } => operation,
                    };
                    if identity != &attempt.identity || operation != &attempt.operation {
                        self.error = Some(FormError::Backend);
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
                        self.local_only =
                            matches!(outcome, DraftSaveOutcome::LocalDraftSaved { .. });
                        self.confirmed = true;
                        self.error = None;
                        self.attempt = None;
                    }
                }
                Err(error) => {
                    self.error = Some(FormError::Runtime(error));
                    attempt.retry_at = Instant::now() + Duration::from_secs(1);
                    if self.closing {
                        self.fallback =
                            Some((self.input, self.connection.command(Command::PersistDrafts)));
                    }
                }
            }
        }
        if !self.patch_error
            && self.patches.is_empty()
            && self.fallback.is_none()
            && self.connection.control.is_open()
            && self.durable != Some(self.input)
        {
            let now = Instant::now();
            let due = self.closing
                || now.duration_since(self.changed_at)
                    >= Duration::from_millis(taypeer_services::AUTOSAVE_DELAY_MILLIS);
            if due
                && self
                    .attempt
                    .as_ref()
                    .is_none_or(|a| a.ticket.is_none() && a.retry_at <= now)
            {
                if self.attempt.is_none() {
                    match taypeer_services::new_operation_id() {
                        Ok(operation) => {
                            self.attempt = Some(Attempt {
                                identity: self.identity.clone(),
                                input: self.input,
                                operation,
                                ticket: None,
                                retry_at: now,
                            })
                        }
                        Err(_) => {
                            self.error = Some(FormError::Backend);
                            self.closing = false;
                            return true;
                        }
                    }
                }
                let attempt = self
                    .attempt
                    .as_mut()
                    .expect("attempt created before submission");
                attempt.ticket = Some(self.connection.command(Command::SaveDraftSnapshot {
                    draft: attempt.identity.draft.clone(),
                    revision: attempt.identity.revision,
                    operation: attempt.operation.clone(),
                }));
                changed = true;
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
            icon_choice(Button::new("group-icon"), &selected, cx)
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
            .text_sm()
            .child(metadata_fields(
                &self.name,
                &self.description,
                invalid_name,
                icon,
                cx,
            ))
            .child(
                div()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr(if !self.settled() {
                        "ui.autosave_pending"
                    } else if self.local_only {
                        "ui.draft_saved"
                    } else if self.input == 0
                        && matches!(self.identity.target, DraftTarget::NewGroup { .. })
                    {
                        "ui.autosave_ready"
                    } else {
                        "ui.autosave_saved"
                    })),
            )
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
fn icon_choice(button: Button, selected: &str, cx: &App) -> Button {
    button
        .w(rems(13.5))
        .px(rems(0.625))
        .bg(cx.theme().button)
        .border_color(cx.theme().border)
        .text_color(cx.theme().foreground)
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
            v_flex()
                .px_6()
                .text_sm()
                .child(metadata_fields(
                    &self.name,
                    &self.description,
                    false,
                    Some(icon_choice(Button::new("group-icon"), "folder", cx).into_any_element()),
                    cx,
                ))
                .child(
                    div()
                        .text_color(cx.theme().muted_foreground)
                        .child(tr("ui.autosave_ready")),
                )
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
    window.open_dialog(cx, move |dialog, window, _| {
        let close = form.clone();
        dialog
            .title(tr(title))
            .width(window.rem_size() * 42.5)
            .p_6()
            .overlay_closable(false)
            .close_button(false)
            .child(form.clone())
            .footer(
                h_flex().justify_end().child(
                    Button::new("close-metadata")
                        .label(tr("close"))
                        .on_click(move |_, _, cx| {
                            close.update(cx, |form, cx| {
                                form.request_close();
                                cx.notify();
                            })
                        }),
                ),
            )
            .on_cancel({
                let close = form.clone();
                move |_, _, cx| {
                    close.update(cx, |form, cx| {
                        form.request_close();
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
