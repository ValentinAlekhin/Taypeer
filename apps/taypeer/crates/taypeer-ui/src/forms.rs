//! Small reusable text forms; each caller supplies its scenario-specific command.

use crate::FormError;
use crate::style::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::{InputEvent, InputState},
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
    input: Entity<InputState>,
    initial: String,
    secret: bool,
}
/// One-shot form completion; invoke on the UI context after the operation resolves.
pub type Done = Box<dyn FnOnce(Result<(), FormError>, &mut Window, &mut App)>;
/// A validated operation started after the form captures its pending state.
pub type Deferred = Box<dyn FnOnce(Done, &mut Window, &mut App)>;
type Submit = Box<dyn Fn(&[String], &mut Window, &mut App) -> Result<Option<Deferred>, FormError>>;
struct TextForm {
    after_save: Option<QuitContinuation>,
    fields: Vec<FormField>,
    submit: Submit,
    error: Option<FormError>,
    busy: bool,
    icon: Option<String>,
    initial_icon: Option<String>,
    _subscriptions: Vec<Subscription>,
}
impl TextForm {
    fn new(
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
                input: input(&value, secret, window, cx),
                initial: value,
                secret,
            })
            .collect();
        let subscriptions = fields
            .iter()
            .map(|field| {
                cx.subscribe(&field.input, |this, _, _: &InputEvent, cx| {
                    this.error = None;
                    cx.notify();
                })
            })
            .collect();
        Self {
            after_save: None,
            fields,
            submit,
            error: None,
            busy: false,
            initial_icon: icon.clone(),
            icon,
            _subscriptions: subscriptions,
        }
    }
    fn dirty(&self, cx: &App) -> bool {
        self.icon != self.initial_icon
            || self
                .fields
                .iter()
                .any(|field| field.input.read(cx).value().as_str() != field.initial)
    }
    fn commit(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        let values = zeroize::Zeroizing::new(
            self.fields
                .iter()
                .map(|field| field.input.read(cx).value().to_string())
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
            .gap_3()
            .text_sm()
            .children(self.fields.iter().map(|item| {
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(
                        div()
                            .id(SharedString::from(format!("form-label-{}", item.label)))
                            .on_click({
                                let focus = item.input.focus_handle(cx);
                                move |_, window, cx| focus.focus(window, cx)
                            })
                            .w(rems(9.))
                            .flex_shrink_0()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(item.label)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(super::clipboard::secret_field(
                                field(&item.input, item.label).disabled(self.busy),
                                &item.input,
                                item.secret,
                            )),
                    )
            }))
            .when_some(self.icon.clone(), |el, selected| {
                el.child(row(
                    "ui.icon",
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
            .when_some(self.error, |el, error| {
                el.child(div().text_color(cx.theme().danger).child(tr(error.key())))
            })
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
    let form = cx.new(|cx| TextForm::new(fields, submit, icon, window, cx));
    if !cx.has_global::<OpenForms>() {
        cx.set_global(OpenForms::default());
    }
    cx.global_mut::<OpenForms>().0.push(form.downgrade());
    let first_input = form
        .read(cx)
        .fields
        .first()
        .map(|field| field.input.clone());
    window.open_dialog(cx, move |dialog, window, _| {
        let save = form.clone();
        let cancel = form.clone();
        dialog
            .title(tr(title))
            .width(window.rem_size() * 38.75)
            .overlay_closable(false)
            .close_button(false)
            .child(form.clone())
            .footer(dialog_actions())
            .on_ok(move |_, window, cx| save.update(cx, |form, cx| form.commit(window, cx)))
            .on_cancel(move |_, window, cx| cancel_form(cancel.clone(), window, cx))
    });
    // Capture the previous window focus before focusing a newly created form.
    if let Some(input) = first_input {
        window.defer(cx, move |window, cx| {
            input.update(cx, |input, cx| input.focus(window, cx));
        });
    }
}

fn cancel_form(form: Entity<TextForm>, window: &mut Window, cx: &mut App) -> bool {
    if form.read(cx).busy {
        return false;
    }
    if !form.read(cx).dirty(cx) {
        return true;
    }
    window.open_dialog(cx, move |dialog, _, _| {
        let save = form.clone();
        dialog
            .title(tr("unsaved"))
            .child(tr("unsaved_body"))
            .close_button(false)
            .overlay_closable(false)
            .footer(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("form-stay")
                            .label(tr("stay"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("form-discard").label(tr("discard")).on_click(
                        |_, window, cx| {
                            window.close_dialog(cx);
                            window.close_dialog(cx);
                        },
                    ))
                    .child(
                        Button::new("form-save")
                            .primary()
                            .label(tr("save"))
                            .on_click(move |_, window, cx| {
                                let success = save.update(cx, |form, cx| form.commit(window, cx));
                                window.close_dialog(cx);
                                if success {
                                    window.close_dialog(cx);
                                }
                            }),
                    ),
            )
    });
    false
}

impl Drop for FormField {
    fn drop(&mut self) {
        use zeroize::Zeroize;
        self.initial.zeroize();
    }
}

/// Continue quitting only after any dirty shared form has been resolved.
pub fn request_quit(continue_quit: QuitContinuation, window: &mut Window, cx: &mut App) {
    let form = cx
        .try_global::<OpenForms>()
        .and_then(|forms| forms.0.iter().rev().find_map(WeakEntity::upgrade));
    let Some(form) = form.filter(|_| window.has_active_dialog(cx)) else {
        window.close_all_dialogs(cx);
        continue_quit(window, cx);
        return;
    };
    if form.read(cx).after_save.is_some() {
        return;
    }
    if form.read(cx).busy {
        form.update(cx, |form, _| form.after_save = Some(continue_quit));
        return;
    }
    if !form.read(cx).dirty(cx) {
        window.close_all_dialogs(cx);
        continue_quit(window, cx);
        return;
    }
    window.open_dialog(cx, move |dialog, _, _| {
        let save = form.clone();
        let discard = continue_quit.clone();
        let continuation = continue_quit.clone();
        dialog
            .title(tr("unsaved"))
            .child(tr("unsaved_body"))
            .close_button(false)
            .overlay_closable(false)
            .footer(
                h_flex()
                    .gap_2()
                    .child(
                        Button::new("quit-stay")
                            .label(tr("stay"))
                            .on_click(|_, window, cx| window.close_dialog(cx)),
                    )
                    .child(Button::new("quit-discard").label(tr("discard")).on_click(
                        move |_, window, cx| {
                            window.close_all_dialogs(cx);
                            discard(window, cx);
                        },
                    ))
                    .child(
                        Button::new("quit-save")
                            .primary()
                            .label(tr("save"))
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                let done = save.update(cx, |form, cx| {
                                    form.after_save = Some(continuation.clone());
                                    let done = form.commit(window, cx);
                                    if !done && !form.busy {
                                        form.after_save = None;
                                    }
                                    done
                                });
                                if done {
                                    window.close_all_dialogs(cx);
                                    continuation(window, cx);
                                }
                            }),
                    ),
            )
    });
}
