//! Small reusable text forms; each caller supplies its scenario-specific command.

use crate::FormError;
use crate::style::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::{InputEvent, InputState, Textarea, TextareaState},
        *,
    },
    *,
};

type QuitContinuation = std::rc::Rc<dyn Fn(&mut Window, &mut App)>;
#[derive(Default)]
struct OpenForms(Vec<WeakEntity<TextForm>>);
impl Global for OpenForms {}

struct FormField {
    label: &'static str,
    control: FormControl,
    secret: bool,
}
enum FormControl {
    Input(Entity<InputState>),
    Description(Entity<TextareaState>),
}
impl FormControl {
    fn value(&self, cx: &App) -> String {
        match self {
            Self::Input(input) => input.read(cx).value().to_string(),
            Self::Description(input) => input.read(cx).value().to_string(),
        }
    }
    fn focus(&self, cx: &App) -> FocusHandle {
        match self {
            Self::Input(input) => input.focus_handle(cx),
            Self::Description(input) => input.focus_handle(cx),
        }
    }
    fn render(
        &self,
        label: &'static str,
        secret: bool,
        busy: bool,
        error: bool,
        cx: &App,
    ) -> AnyElement {
        match self {
            Self::Input(input) => super::clipboard::secret_field(
                field(input, label)
                    .disabled(busy)
                    .when(secret, |field| {
                        let state = input.clone();
                        field.suffix(
                            icon_button(
                                SharedString::from(format!("form-reveal-{label}")),
                                if input.read(cx).presentation().is_masked() {
                                    "eye"
                                } else {
                                    "eye-off"
                                },
                                "show",
                            )
                            .disabled(busy)
                            .on_click(move |_, window, cx| {
                                state.update(cx, |input, cx| input.toggle_masked(window, cx));
                            }),
                        )
                    })
                    .when(error, |field| {
                        field.border_1().border_color(cx.theme().danger)
                    }),
                input,
                secret,
            ),
            Self::Description(input) => div()
                .id(SharedString::from(format!("field-{label}")))
                .test_support()
                .child(
                    Textarea::new(input)
                        .aria_label(tr(label))
                        .disabled(busy)
                        .h(rems(4.))
                        .appearance(false)
                        .bordered(false)
                        .bg(cx.theme().title_bar)
                        .rounded(rems(0.25))
                        .px_0()
                        .py_0()
                        .font_weight(FontWeight::NORMAL)
                        .text_size(rems(13. / 16.))
                        .line_height(rems(1.25)),
                )
                .into_any_element(),
        }
    }
}
/// One-shot form completion; invoke on the UI context after the operation resolves.
pub type Done = Box<dyn FnOnce(Result<(), FormError>, &mut Window, &mut App)>;
/// A validated operation started after the form captures its pending state.
pub type Deferred = Box<dyn FnOnce(Done, &mut Window, &mut App)>;
type Submit = Box<dyn Fn(&[String], &mut Window, &mut App) -> Result<Option<Deferred>, FormError>>;

/// A modal field row; the dialog owns its outer inset, and the label focuses the retained control.
pub fn form_row(label: &str, focus: Option<FocusHandle>, value: impl IntoElement, cx: &App) -> Div {
    h_flex()
        .min_h(rems(2.75))
        .gap_4()
        .items_start()
        .font_weight(FontWeight::NORMAL)
        .child(
            div()
                .id(SharedString::from(format!("form-label-{label}")))
                .test_support()
                .w(rems(9.))
                .flex_shrink_0()
                .pt(rems(0.375))
                .text_size(rems(13. / 16.))
                .line_height(rems(1.25))
                .text_color(cx.theme().muted_foreground)
                .child(tr(label))
                .when_some(focus, |label, focus| {
                    label.on_click(move |_, window, cx| focus.focus(window, cx))
                }),
        )
        .child(div().flex_1().min_w_0().child(value))
}
struct TextForm {
    title: &'static str,
    after_save: Option<QuitContinuation>,
    fields: Vec<FormField>,
    submit: Submit,
    error: Option<FormError>,
    busy: bool,
    icon: Option<String>,
    _subscriptions: Vec<Subscription>,
}
impl TextForm {
    fn error_field(&self) -> Option<&'static str> {
        let labels = self.fields.iter().map(|field| field.label);
        match self.error? {
            FormError::RequiredName | FormError::DuplicateAttribute => labels
                .clone()
                .find(|label| matches!(*label, "name" | "ui.device_name" | "ui.key")),
            FormError::PasswordConfirmation => labels
                .clone()
                .find(|label| matches!(*label, "confirm_password" | "ui.new_password_confirm")),
            FormError::InvalidNumber => {
                let mut numbers = labels.filter(|label| {
                    matches!(
                        *label,
                        "ui.seconds"
                            | "ui.kdf_seconds"
                            | "ui.attachment_limit"
                            | "ui.total_attachment_limit"
                    )
                });
                let first = numbers.next();
                // Aggregate validation does not identify which numeric control failed.
                numbers.next().is_none().then_some(first).flatten()
            }
            _ => None,
        }
    }
    fn new(
        title: &'static str,
        fields: Vec<(&'static str, String, bool)>,
        submit: Submit,
        icon: Option<String>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fields: Vec<_> = fields
            .into_iter()
            .map(|(label, value, secret)| FormField {
                label,
                control: if label == "ui.description" && !secret {
                    FormControl::Description(cx.new(|cx| {
                        let mut state = TextareaState::new(window, cx);
                        state.set_value(value, window, cx);
                        state
                    }))
                } else {
                    FormControl::Input(input(&value, secret, window, cx))
                },
                secret,
            })
            .collect();
        let subscriptions = fields
            .iter()
            .map(|field| match &field.control {
                FormControl::Input(input) => cx.subscribe(input, |this, _, _: &InputEvent, cx| {
                    this.error = None;
                    cx.notify();
                }),
                FormControl::Description(input) => {
                    cx.subscribe(input, |this, _, _: &InputEvent, cx| {
                        this.error = None;
                        cx.notify();
                    })
                }
            })
            .collect();
        Self {
            title,
            after_save: None,
            fields,
            submit,
            error: None,
            busy: false,
            icon,
            _subscriptions: subscriptions,
        }
    }
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        let values = zeroize::Zeroizing::new(
            self.fields
                .iter()
                .map(|field| field.control.value(cx))
                .chain(self.icon.iter().cloned())
                .collect::<Vec<_>>(),
        );
        match (self.submit)(&values, window, cx) {
            Ok(None) => true,
            Ok(Some(submit)) => {
                self.busy = true;
                let form = cx.entity().downgrade();
                submit(
                    Box::new(move |result, window, cx| {
                        // A validation refusal may complete synchronously inside commit.
                        window.defer(cx, move |window, cx| {
                            let alive = form.update(cx, |form, cx| {
                                form.busy = false;
                                form.error = result.err().filter(|e| *e != FormError::Canceled);
                                cx.notify();
                            });
                            if alive.is_ok() && result.is_ok() {
                                window.close_dialog(cx);
                                if let Ok(Some(continue_quit)) =
                                    form.update(cx, |form, _| form.after_save.take())
                                {
                                    window.close_all_dialogs(cx);
                                    continue_quit(window, cx);
                                }
                            } else {
                                let _ = form.update(cx, |form, _| form.after_save = None);
                            }
                        });
                    }),
                    window,
                    cx,
                );
                cx.notify();
                false
            }
            Err(error) => {
                self.error = Some(error);
                cx.notify();
                false
            }
        }
    }
}
impl Render for TextForm {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        v_flex()
            .capture_key_down(|_, _, cx| taypeer_desktop_platform::activity(cx))
            .capture_any_mouse_down(|_, _, cx| taypeer_desktop_platform::activity(cx))
            .text_sm()
            .children(self.fields.iter().map(|item| {
                let error = self
                    .error
                    .filter(|_| self.error_field() == Some(item.label));
                let height = if self.title == "create_db" {
                    match item.label {
                        "name" => 3.,
                        "ui.description" => 5.,
                        "password" => 4.625,
                        "confirm_password" => 3.125,
                        "ui.kdf_seconds" => 13.75,
                        _ => 2.75,
                    }
                } else {
                    2.75
                };
                div().min_h(rems(height)).child(form_row(
                    item.label,
                    Some(item.control.focus(cx)),
                    v_flex()
                        .gap_1()
                        .child(item.control.render(
                            item.label,
                            item.secret,
                            self.busy,
                            error.is_some(),
                            cx,
                        ))
                        .when(
                            self.title == "create_db" && item.label == "ui.kdf_seconds",
                            |column| {
                                column
                                    .child(
                                        div()
                                            .mt_3()
                                            .min_h(rems(3.))
                                            .text_size(rems(13. / 16.))
                                            .line_height(rems(1.25))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("ui.kdf_calibration_hint")),
                                    )
                                    .child(
                                        div()
                                            .mt_6()
                                            .min_h(rems(1.5))
                                            .text_size(rems(13. / 16.))
                                            .line_height(rems(1.25))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("ui.internal_working_copy")),
                                    )
                                    .child(
                                        div()
                                            .mt_8()
                                            .min_h(rems(3.))
                                            .text_size(rems(13. / 16.))
                                            .line_height(rems(1.25))
                                            .text_color(cx.theme().muted_foreground)
                                            .child(tr("ui.password_recovery_hint")),
                                    )
                            },
                        )
                        .when_some(error, |el, error| {
                            el.child(
                                div()
                                    .id(SharedString::from(format!("form-error-{}", item.label)))
                                    .test_support()
                                    .aria_label(tr(error.key()))
                                    .text_xs()
                                    .text_color(cx.theme().danger)
                                    .child(tr(error.key())),
                            )
                        }),
                    cx,
                ))
            }))
            .when_some(self.icon.clone(), |el, selected| {
                el.child(form_row(
                    "ui.icon",
                    None,
                    Button::new("group-icon")
                        .icon(icon(&selected))
                        .disabled(self.busy)
                        .label(tr("ui.choose_icon"))
                        .on_click(cx.listener(move |this, _, window, cx| {
                            let form = cx.entity().downgrade();
                            super::icons::choose(
                                this.icon.clone().unwrap_or_default(),
                                move |name, cx| {
                                    // The picker may outlive its form when the workspace locks.
                                    let _ = form.update(cx, |form, cx| {
                                        form.icon = Some(name.into());
                                        cx.notify();
                                    });
                                },
                                window,
                                cx,
                            )
                        })),
                    cx,
                ))
            })
            .when_some(
                self.error.filter(|_| self.error_field().is_none()),
                |el, error| {
                    el.child(
                        div()
                            .id("form-error")
                            .test_support()
                            .aria_label(tr(error.key()))
                            .text_color(cx.theme().danger)
                            .child(tr(error.key())),
                    )
                },
            )
    }
}

/// Open a retained text form; submission owns validation and asynchronous completion.
pub fn text_form(
    title: &'static str,
    fields: Vec<(&'static str, String, bool)>,
    submit: Submit,
    window: &mut Window,
    cx: &mut App,
) {
    text_form_with_icon(title, fields, submit, None, window, cx);
}

/// Open a text form with an explicit icon slot and the same completion contract.
pub fn text_form_with_icon(
    title: &'static str,
    fields: Vec<(&'static str, String, bool)>,
    submit: Submit,
    icon: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| TextForm::new(title, fields, submit, icon, window, cx));
    if !cx.has_global::<OpenForms>() {
        cx.set_global(OpenForms::default());
    }
    cx.global_mut::<OpenForms>().0.push(form.downgrade());
    let first_input = form
        .read(cx)
        .fields
        .first()
        .map(|field| field.control.focus(cx));
    window.open_dialog(cx, move |dialog, window, _| {
        let save = form.clone();
        let cancel = form.clone();
        dialog
            .title(tr(title))
            .width(window.rem_size() * if title == "create_db" { 45. } else { 38.75 })
            .p_6()
            .overlay_closable(false)
            .close_button(false)
            .child(form.clone())
            .footer(form_actions(title))
            .on_ok(move |_, window, cx| save.update(cx, |form, cx| form.commit(window, cx)))
            .on_cancel(move |_, window, cx| cancel_form(cancel.clone(), window, cx))
    });
    // Capture the previous window focus before focusing a newly created form.
    if let Some(focus) = first_input {
        window.defer(cx, move |window, cx| {
            focus.focus(window, cx);
        });
    }
}

fn form_actions(title: &str) -> AnyElement {
    let label = match title {
        "create_db" => "create_db",
        "ui.clone_group" => "ui.clone_group",
        "ui.add_attribute" => "ui.add_attribute",
        "ui.device_name" | "ui.auto_lock" | "ui.clipboard_timeout" | "sync.relay_custom" => "save",
        _ => "ui.confirm",
    };
    h_flex()
        .justify_end()
        .gap_2()
        .child(
            Button::new("dialog-cancel")
                .label(tr("cancel"))
                .on_click(|_, window, cx| window.dispatch_action(Box::new(dialog::Cancel), cx)),
        )
        .child(
            Button::new("dialog-confirm")
                .primary()
                .label(tr(label))
                .on_click(|_, window, cx| {
                    window.dispatch_action(Box::new(dialog::Confirm { secondary: false }), cx)
                }),
        )
        .into_any_element()
}

#[cfg(feature = "component-rendering")]
#[derive(Clone, Copy)]
/// Public synthetic retained form layouts, independent of runtime stores.
pub enum FormPreviewCase {
    /// Initial creation controls; both password inputs are empty.
    Database,
    /// Validation error remains beside the required name.
    DatabaseError,
}

#[cfg(feature = "component-rendering")]
/// Mount the real form content and action builder with inert synthetic submission.
pub fn form_preview(case: FormPreviewCase, window: &mut Window, cx: &mut App) -> AnyView {
    struct Preview {
        form: Entity<TextForm>,
    }
    impl Render for Preview {
        fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
            div().px_6().child(self.form.clone())
        }
    }
    let (title, fields, icon) = match case {
        FormPreviewCase::Database | FormPreviewCase::DatabaseError => (
            "create_db",
            vec![
                (
                    "name",
                    if matches!(case, FormPreviewCase::DatabaseError) {
                        String::new()
                    } else {
                        "Рабочая".into()
                    },
                    false,
                ),
                (
                    "ui.description",
                    "Личные проекты и рабочие сервисы".into(),
                    false,
                ),
                ("password", String::new(), true),
                ("confirm_password", String::new(), true),
                ("ui.kdf_seconds", "1.0".into(), false),
            ],
            None,
        ),
    };
    let form = cx.new(|cx| {
        let mut form = TextForm::new(
            title,
            fields,
            Box::new(|_, _, _| Ok(None)),
            icon,
            window,
            cx,
        );
        if matches!(case, FormPreviewCase::DatabaseError) {
            form.error = Some(FormError::RequiredName);
        }
        form
    });
    cx.new(|_| Preview { form }).into()
}

fn cancel_form(form: Entity<TextForm>, _: &mut Window, cx: &mut App) -> bool {
    !form.read(cx).busy
}

/// Finish an already submitted security command, then delegate encrypted form flushing.
pub fn request_quit(continue_quit: QuitContinuation, window: &mut Window, cx: &mut App) {
    let form = cx
        .try_global::<OpenForms>()
        .and_then(|forms| forms.0.iter().rev().find_map(WeakEntity::upgrade));
    if let Some(form) = form.filter(|form| window.has_active_dialog(cx) && form.read(cx).busy) {
        form.update(cx, |form, _| form.after_save = Some(continue_quit));
        return;
    }
    window.close_all_dialogs(cx);
    continue_quit(window, cx);
}
