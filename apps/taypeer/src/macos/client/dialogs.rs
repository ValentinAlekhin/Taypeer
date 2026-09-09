//! Named forms, dirty-navigation confirmation and explicit draft restoration.

use super::{Client, Form, Navigation};
use crate::macos::common::{input, tr};
use gpui_kit::component::input::Input;
use gpui_kit::component::{button::*, *};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl Client {
    pub(super) fn open_form(
        &mut self,
        form: Form,
        value: &str,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        if self
            .editor
            .as_ref()
            .is_some_and(|editor| editor.draft.dirty)
        {
            self.pending = Some(Navigation::Form(form, value.into()));
            self.modal_focus.focus(window, cx);
            cx.notify();
            return;
        }
        if self.editor.is_some() {
            if let Some(token) = &self.session
                && self.service.cancel_draft(token).is_err()
            {
                self.error = Some("error");
                cx.notify();
                return;
            }
            self.editor = None;
        }
        self.form_input = input(value, false, window, cx);
        self.form = Some(form);
        self.bind_prompt_inputs(window, cx);
        self.form_input
            .update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn commit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let name = self.form_input.read(cx).value().to_string();
        let success = match self.form.clone() {
            Some(Form::Database) => match self.service.create_database(name) {
                Ok(id) => {
                    self.clear_content(window, cx);
                    self.session = None;
                    self.database = Some(id);
                    true
                }
                Err(_) => false,
            },
            Some(Form::Group(parent)) => self.session.clone().as_ref().is_some_and(|token| {
                match self.service.create_group(token, name, parent) {
                    Ok(reply) if self.accepts(&reply.session) => {
                        self.group = Some(reply.value.id);
                        true
                    }
                    _ => false,
                }
            }),
            Some(Form::Rename(id)) => self
                .session
                .as_ref()
                .is_some_and(|token| self.service.update_group(token, &id, name).is_ok()),
            None => false,
        };
        if success {
            self.form = None;
            self.root_focus.focus(window, cx);
            self.error = None;
        } else {
            self.error = Some("error");
        }
        cx.notify();
    }

    pub(super) fn overlay(&self, cx: &mut Context<Self>) -> Option<AnyElement> {
        let content =
            if self.settings {
                self.settings_content(cx)
            } else if self.pending.is_some() {
                v_flex()
                    .gap_4()
                    .child(div().text_lg().child(tr("unsaved")))
                    .child(tr("unsaved_body"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("nav-save")
                                    .label(tr("save"))
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.save_and_navigate(window, cx);
                                    })),
                            )
                            .child(Button::new("nav-discard").label(tr("discard")).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.discard_and_navigate(window, cx);
                                }),
                            ))
                            .child(Button::new("nav-stay").label(tr("stay")).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.stay_in_editor(window, cx);
                                }),
                            )),
                    )
                    .into_any_element()
            } else if self.restore {
                v_flex()
                    .gap_4()
                    .child(tr("restore_body"))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("restore-draft")
                                    .label(tr("restore"))
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.restore_draft(window, cx);
                                    })),
                            )
                            .child(
                                Button::new("discard-restored")
                                    .label(tr("discard"))
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.discard_restored(window, cx);
                                    })),
                            ),
                    )
                    .into_any_element()
            } else if let Some(form) = &self.form {
                v_flex()
                    .gap_4()
                    .child(div().text_lg().child(tr(match form {
                        Form::Database => "create_db",
                        Form::Group(_) => "add_group",
                        Form::Rename(_) => "edit_group",
                    })))
                    .child(Input::new(&self.form_input).aria_label(tr("name")))
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("form-commit")
                                    .label(tr(if matches!(form, Form::Rename(_)) {
                                        "save"
                                    } else {
                                        "create"
                                    }))
                                    .primary()
                                    .on_click(cx.listener(|this, _, window, cx| {
                                        this.commit_form(window, cx)
                                    })),
                            )
                            .child(Button::new("form-cancel").label(tr("cancel")).on_click(
                                cx.listener(|this, _, window, cx| {
                                    this.cancel_form(window, cx);
                                }),
                            )),
                    )
                    .into_any_element()
            } else {
                return None;
            };
        Some(
            div()
                .absolute()
                .occlude()
                .inset_0()
                .bg(gpui_kit::rgba(0x00000066))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    v_flex()
                        .w(px(540.))
                        .p_6()
                        .gap_3()
                        .rounded_lg()
                        .border_1()
                        .border_color(cx.theme().border)
                        .bg(cx.theme().background)
                        .child(content)
                        .when_some(self.error, |el, error| {
                            el.child(div().text_color(cx.theme().danger).child(tr(error)))
                        })
                        .focus_trap("modal-focus", &self.modal_focus),
                )
                .into_any_element(),
        )
    }

    pub(super) fn cancel_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        self.root_focus.focus(window, cx);
        self.form = None;
        self.error = None;
        cx.notify();
    }
}
