//! Small reusable text forms; each caller supplies its scenario-specific command.

use super::{style::*, workspace::WorkspaceStore};
use crate::ui_state::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        input::{InputEvent, InputState},
        *,
    },
    *,
};

struct FormField {
    label: &'static str,
    input: Entity<InputState>,
    initial: String,
}
type Submit = Box<dyn Fn(&[String], &mut Window, &mut App) -> Result<(), FormError>>;
pub(super) struct TextForm {
    fields: Vec<FormField>,
    submit: Submit,
    error: Option<FormError>,
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
            fields,
            submit,
            error: None,
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
        let values = self
            .fields
            .iter()
            .map(|field| field.input.read(cx).value().to_string())
            .chain(self.icon.iter().cloned())
            .collect::<Vec<_>>();
        match (self.submit)(&values, window, cx) {
            Ok(()) => true,
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
            .gap_3()
            .text_sm()
            .children(self.fields.iter().map(|item| {
                h_flex()
                    .gap_4()
                    .items_center()
                    .child(
                        div()
                            .w(rems(9.))
                            .flex_shrink_0()
                            .text_color(cx.theme().muted_foreground)
                            .child(tr(item.label)),
                    )
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .child(field(&item.input, item.label)),
                    )
            }))
            .when_some(self.icon.clone(), |el, selected| {
                el.child(row(
                    "ui.icon",
                    Button::new("group-icon")
                        .icon(icon(&selected))
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

pub(super) fn text_form(
    title: &'static str,
    fields: Vec<(&'static str, String, bool)>,
    submit: Submit,
    window: &mut Window,
    cx: &mut App,
) {
    text_form_with_icon(title, fields, submit, None, window, cx);
}

fn text_form_with_icon(
    title: &'static str,
    fields: Vec<(&'static str, String, bool)>,
    submit: Submit,
    icon: Option<String>,
    window: &mut Window,
    cx: &mut App,
) {
    let form = cx.new(|cx| TextForm::new(fields, submit, icon, window, cx));
    let first_input = form
        .read(cx)
        .fields
        .first()
        .map(|field| field.input.clone());
    window.open_dialog(cx, move |dialog, _, _| {
        let save = form.clone();
        let cancel = form.clone();
        dialog
            .title(tr(title))
            .width(px(620.))
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

pub(super) fn choose_sample(store: Entity<WorkspaceStore>, window: &mut Window, cx: &mut App) {
    window.open_dialog(cx, move |dialog, _, cx| {
        let catalog = store.read(cx).catalog().read(cx);
        dialog
            .title(tr("ui.open_sample"))
            .width(px(520.))
            .child(
                v_flex()
                    .gap_2()
                    .child(tr("ui.sample_hint"))
                    .children(catalog.databases().map(|db| {
                        let target = store.clone();
                        let id = db.id;
                        Button::new(("sample", id.0))
                            .ghost()
                            .justify_start()
                            .icon(icon("file-key-2"))
                            .label(db.name.clone())
                            .on_click(move |_, window, cx| {
                                window.close_dialog(cx);
                                target.update(cx, |store, cx| {
                                    store.navigate(Destination::Database(id), window, cx)
                                });
                            })
                    })),
            )
            .footer(
                Button::new("close-samples")
                    .label(tr("cancel"))
                    .on_click(|_, window, cx| window.close_dialog(cx)),
            )
    });
}

pub(super) fn database(
    store: Entity<WorkspaceStore>,
    id: Option<DatabaseId>,
    window: &mut Window,
    cx: &mut App,
) {
    let (name, description) = id
        .and_then(|id| store.read(cx).catalog().read(cx).database(id))
        .map(|db| (db.name.clone(), db.description.clone()))
        .unwrap_or_default();
    let mut fields = vec![
        ("name", name, false),
        ("ui.description", description, false),
    ];
    if id.is_none() {
        fields.extend([
            ("password", String::new(), true),
            ("confirm_password", String::new(), true),
        ]);
    }
    text_form(
        if id.is_some() {
            "ui.database_info"
        } else {
            "create_db"
        },
        fields,
        Box::new(move |values, window, cx| {
            if id.is_none() && (values[2].is_empty() || values[2] != values[3]) {
                return Err(FormError::PasswordConfirmation);
            }
            let catalog = store.read(cx).catalog().clone();
            let db = catalog.update(cx, |catalog, cx| {
                let result = if let Some(id) = id {
                    catalog
                        .update_database(id, values[0].clone(), values[1].clone())
                        .map(|()| id)
                } else {
                    catalog.create_database(values[0].clone(), values[1].clone())
                };
                if result.is_ok() {
                    cx.notify();
                }
                result
            })?;
            if id.is_none() {
                let store = store.clone();
                // Let Kit dismiss this form before presenting an unsaved draft dialog.
                window.defer(cx, move |window, cx| {
                    store.update(cx, |store, cx| store.open_created(db, window, cx));
                });
            }
            Ok(())
        }),
        window,
        cx,
    );
}

pub(super) fn group(
    store: Entity<WorkspaceStore>,
    id: Option<GroupId>,
    parent: Option<GroupId>,
    window: &mut Window,
    cx: &mut App,
) {
    let state = store.read(cx);
    if !state.state().is_unlocked() {
        return;
    }
    let Some(db) = state.state().database else {
        return;
    };
    let group = id.and_then(|id| {
        state
            .catalog()
            .read(cx)
            .database(db)?
            .groups
            .iter()
            .find(|g| g.id == id)
    });
    let fields = vec![
        (
            "name",
            group.map(|g| g.name.clone()).unwrap_or_default(),
            false,
        ),
        (
            "ui.description",
            group.map(|g| g.description.clone()).unwrap_or_default(),
            false,
        ),
    ];
    let group_icon = group
        .map(|g| g.icon.clone())
        .unwrap_or_else(|| "folder".into());
    text_form_with_icon(
        if id.is_some() {
            "edit_group"
        } else {
            "add_group"
        },
        fields,
        Box::new(move |values, window, cx| {
            let catalog = store.read(cx).catalog().clone();
            let group = catalog.update(cx, |catalog, cx| {
                let result = catalog.save_group(
                    db,
                    id,
                    parent,
                    values[0].clone(),
                    values[1].clone(),
                    values[2].clone(),
                );
                if result.is_ok() {
                    cx.notify();
                }
                result
            })?;
            let store = store.clone();
            window.defer(cx, move |window, cx| {
                store.update(cx, |store, cx| {
                    store.navigate(Destination::Group(group), window, cx)
                });
            });
            Ok(())
        }),
        Some(group_icon),
        window,
        cx,
    );
}

pub(super) fn attribute(
    editor: Entity<EditorStore>,
    index: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) {
    let attribute = index.and_then(|index| editor.read(cx).content().attributes.get(index));
    let fields = vec![
        (
            "ui.key",
            attribute.map(|a| a.key.clone()).unwrap_or_default(),
            false,
        ),
        (
            "value",
            attribute.map(|a| a.value.clone()).unwrap_or_default(),
            attribute.is_some_and(|a| a.protected),
        ),
    ];
    text_form(
        "ui.attribute",
        fields,
        Box::new(move |values, _, cx| {
            require_name(&values[0])?;
            if editor
                .read(cx)
                .content()
                .attributes
                .iter()
                .enumerate()
                .any(|(i, a)| Some(i) != index && a.key == values[0])
            {
                return Err(FormError::DuplicateAttribute);
            }
            editor.update(cx, |editor, cx| {
                editor.edit(|content| {
                    if let Some(index) = index {
                        if let Some(attribute) = content.attributes.get_mut(index) {
                            attribute.key = values[0].clone();
                            attribute.value = values[1].clone();
                        }
                    } else {
                        content.attributes.push(Attribute {
                            key: values[0].clone(),
                            value: values[1].clone(),
                            protected: false,
                        });
                    }
                });
                cx.notify();
            });
            Ok(())
        }),
        window,
        cx,
    );
}

pub(super) fn attachment(
    editor: Entity<EditorStore>,
    index: Option<usize>,
    window: &mut Window,
    cx: &mut App,
) {
    let name = index
        .and_then(|index| editor.read(cx).content().attachments.get(index))
        .map(|a| a.name.clone())
        .unwrap_or_else(|| "public-sample.txt".into());
    text_form(
        "ui.attachment",
        vec![("name", name, false)],
        Box::new(move |values, _, cx| {
            require_name(&values[0])?;
            editor.update(cx, |editor, cx| {
                editor.edit(|content| {
                    if let Some(index) = index {
                        if let Some(attachment) = content.attachments.get_mut(index) {
                            attachment.name = values[0].clone();
                        }
                    } else {
                        content.attachments.push(Attachment {
                            name: values[0].clone(),
                            bytes: 20480,
                        });
                    }
                });
                cx.notify();
            });
            Ok(())
        }),
        window,
        cx,
    );
}

pub(super) fn image_sample(editor: Entity<EditorStore>, window: &mut Window, cx: &mut App) {
    text_form(
        "ui.image_url",
        vec![("url", "https://public.example.test/icon.png".into(), false)],
        Box::new(move |_, _, cx| {
            editor.update(cx, |editor, cx| {
                editor.edit(|content| content.icon = "globe".into());
                cx.notify();
            });
            Ok(())
        }),
        window,
        cx,
    );
}

pub(super) fn color(
    editor: Entity<EditorStore>,
    background: bool,
    window: &mut Window,
    cx: &mut App,
) {
    let color = if background {
        editor.read(cx).content().background
    } else {
        editor.read(cx).content().foreground
    };
    text_form(
        if background {
            "ui.background_color"
        } else {
            "ui.foreground_color"
        },
        vec![(
            "ui.hex_color",
            color.map(|c| format!("#{c:06x}")).unwrap_or_default(),
            false,
        )],
        Box::new(move |values, _, cx| {
            let value = &values[0];
            let color = if value.is_empty() {
                None
            } else {
                let raw = value.strip_prefix('#').unwrap_or(value);
                if raw.len() != 6 {
                    return Err(FormError::InvalidColor);
                }
                Some(u32::from_str_radix(raw, 16).map_err(|_| FormError::InvalidColor)?)
            };
            editor.update(cx, |editor, cx| {
                editor.edit(|content| {
                    if background {
                        content.background = color
                    } else {
                        content.foreground = color
                    }
                });
                cx.notify();
            });
            Ok(())
        }),
        window,
        cx,
    );
}
