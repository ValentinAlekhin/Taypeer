//! Named forms, dirty-navigation confirmation and explicit draft restoration.

use super::{Client, Form, Navigation};
use crate::macos::common::{input, tr};
use gpui_kit::component::input::{Input, InputContentType};
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
        if self.busy {
            return;
        }
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
            self.perform_navigation(Navigation::Form(form, value.into()), window, cx);
            return;
        }
        self.file_password = input("", true, window, cx);
        self.file_confirmation = input("", true, window, cx);
        self.form_input = input(value, false, window, cx);
        self.form = Some(form);
        self.bind_prompt_inputs(window, cx);
        let focus = if matches!(self.form, Some(Form::OpenFile(_))) {
            &self.file_password
        } else {
            &self.form_input
        };
        focus.update(cx, |state, cx| state.focus(window, cx));
        cx.notify();
    }

    pub(super) fn commit_form(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        if self.demo_mode && matches!(self.form, Some(Form::Database)) {
            let name = self.form_input.read(cx).value().to_string();
            match self.service.create_database(name) {
                Ok(id) => {
                    self.clear_content(window, cx);
                    self.session = None;
                    self.database = Some(id);
                    self.error = None;
                }
                Err(_) => self.error = Some("error"),
            }
            cx.notify();
            return;
        }
        if matches!(self.form, Some(Form::Database | Form::OpenFile(_))) {
            self.commit_file_form(window, cx);
            return;
        }
        let Some(token) = self.session.clone() else {
            return;
        };
        let name = self.form_input.read(cx).value().to_string();
        let form = self.form.clone();
        self.run_io(
            window,
            cx,
            move |service| match form {
                Some(Form::Group(parent)) => service
                    .create_group(&token, name, parent)
                    .map(|reply| Some(reply.value.id)),
                Some(Form::Rename(id)) => service.update_group(&token, &id, name).map(|_| None),
                _ => Err(taypeer_services::ServiceError::InvalidContext),
            },
            |this, result, window, cx| match result {
                Ok(group) => {
                    if let Some(group) = group {
                        this.group = Some(group);
                    }
                    this.form = None;
                    this.root_focus.focus(window, cx);
                    this.error = None;
                }
                Err(error) => this.error = Some(super::files::file_error(error)),
            },
        );
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
                        Form::OpenFile(_) => "open_db",
                        Form::Group(_) => "add_group",
                        Form::Rename(_) => "edit_group",
                    })))
                    .when(!matches!(form, Form::OpenFile(_)), |el| {
                        el.child(Input::new(&self.form_input).aria_label(tr("name")))
                    })
                    .when(
                        matches!(form, Form::OpenFile(_))
                            || (!self.demo_mode && matches!(form, Form::Database)),
                        |el| {
                            el.child(tr("password")).child(
                                Input::new(&self.file_password)
                                    .aria_label(tr("password"))
                                    .content_type(InputContentType::Password)
                                    .mask_toggle(),
                            )
                        },
                    )
                    .when(!self.demo_mode && matches!(form, Form::Database), |el| {
                        el.child(tr("confirm_password")).child(
                            Input::new(&self.file_confirmation)
                                .aria_label(tr("confirm_password"))
                                .content_type(InputContentType::Password),
                        )
                    })
                    .child(
                        h_flex()
                            .gap_2()
                            .child(
                                Button::new("form-commit")
                                    .label(tr(if matches!(form, Form::OpenFile(_)) {
                                        "open_db"
                                    } else if matches!(form, Form::Rename(_)) {
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
        self.file_password = input("", true, window, cx);
        self.file_confirmation = input("", true, window, cx);
        self.error = None;
        cx.notify();
    }
}
