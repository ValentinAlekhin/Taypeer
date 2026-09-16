//! Editable entry tabs. The draft owns values; these widgets adapt native input.

use super::{forms, style::*, workspace::WorkspaceStore};
use crate::ui_state::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        checkbox::Checkbox,
        input::{InputEvent, InputState, Textarea, TextareaState},
        *,
    },
    *,
};

pub(super) struct EditorView {
    store: Entity<WorkspaceStore>,
    editor: Entity<EditorStore>,
    fields: Vec<(EntryField, Entity<InputState>)>,
    notes: Entity<TextareaState>,
    foreground: Entity<color_picker::ColorPickerState>,
    background: Entity<color_picker::ColorPickerState>,
    _subscriptions: Vec<Subscription>,
}
impl EditorView {
    pub fn new(
        store: Entity<WorkspaceStore>,
        editor: Entity<EditorStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let content = editor.read(cx).content().clone();
        let fields: Vec<_> = EntryField::ALL
            .into_iter()
            .filter(|f| *f != EntryField::Notes)
            .map(|f| {
                (
                    f,
                    input(f.value(&content), f == EntryField::Password, window, cx),
                )
            })
            .collect();
        if content.has_password
            && let Some((_, password)) = fields
                .iter()
                .find(|(field, _)| *field == EntryField::Password)
        {
            password.update(cx, |input, cx| {
                input.set_placeholder("••••••••••", window, cx)
            });
        }
        let notes = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx);
            state.set_value(content.notes.clone(), window, cx);
            state
        });
        let mut subscriptions = vec![
            cx.observe_in(&editor, window, |this, editor, window, cx| {
                let content = editor.read(cx).content().clone();
                for (field, input) in &this.fields {
                    let value = field.value(&content);
                    if input.read(cx).value().as_str() != value {
                        input.update(cx, |input, cx| {
                            input.set_value(value.to_owned(), window, cx)
                        });
                    }
                }
                if this.notes.read(cx).value().as_str() != content.notes {
                    this.notes.update(cx, |input, cx| {
                        input.set_value(content.notes.clone(), window, cx)
                    });
                }
                cx.notify();
            }),
            cx.observe(&store, |_, _, cx| cx.notify()),
        ];
        for (field, input) in &fields {
            let editor = editor.clone();
            let field = *field;
            subscriptions.push(
                cx.subscribe(input, move |_, input, event: &InputEvent, cx| {
                    if matches!(event, InputEvent::Change) {
                        let value = input.read(cx).value().to_string();
                        editor.update(cx, |editor, cx| {
                            editor.edit(|content| field.set(content, value));
                            cx.notify();
                        });
                    }
                }),
            );
        }
        subscriptions.push(cx.subscribe(&notes, {
            let editor = editor.clone();
            move |_, input, event: &InputEvent, cx| {
                if matches!(event, InputEvent::Change) {
                    let value = input.read(cx).value().to_string();
                    editor.update(cx, |editor, cx| {
                        editor.edit(|content| content.notes = value);
                        cx.notify();
                    });
                }
            }
        }));
        if store.read(cx).state().tab == EntryTab::Overview
            && let Some((_, input)) = fields.first()
        {
            input.update(cx, |input, cx| input.focus(window, cx));
        }
        let foreground = Self::color_picker(&editor, false, window, cx, &mut subscriptions);
        let background = Self::color_picker(&editor, true, window, cx, &mut subscriptions);
        Self {
            foreground,
            background,
            store,
            editor,
            fields,
            notes,
            _subscriptions: subscriptions,
        }
    }
    fn overview(&self, cx: &mut Context<Self>) -> AnyElement {
        let mut result = v_flex();
        for (entry_field, input) in &self.fields {
            let mut control = h_flex().gap_1().child(
                div()
                    .flex_1()
                    .min_w_0()
                    .child(super::clipboard::secret_field(
                        field(input, entry_field.key())
                            .disabled(!self.editor.read(cx).editable())
                            .appearance(false)
                            .when(*entry_field == EntryField::Password, |input| {
                                input.mask_toggle()
                            }),
                        input,
                        *entry_field == EntryField::Password,
                    )),
            );
            if *entry_field == EntryField::Password {
                control =
                    control.child(icon_button("generator", "dice-5", "ui.generator").on_click(
                        cx.listener(|this, _, window, cx| {
                            super::inspector::open_generator(this.editor.clone(), window, cx)
                        }),
                    ));
            }
            if *entry_field == EntryField::Password {
                control = control
                    .child(
                        icon_button("load-password", "eye", "ui.reveal_current")
                            .disabled(self.editor.read(cx).busy())
                            .on_click(cx.listener(|this, _, _, cx| {
                                this.editor.update(cx, |editor, cx| {
                                    editor.reveal(None);
                                    cx.notify();
                                });
                            })),
                    )
                    .child(
                        icon_button("clear-password", "x", "ui.clear_value").on_click(cx.listener(
                            |this, _, _, cx| {
                                this.editor.update(cx, |editor, cx| {
                                    editor.clear_field(EntryField::Password);
                                    cx.notify();
                                });
                            },
                        )),
                    );
            }
            if *entry_field == EntryField::Url {
                control = control.child(icon_button("favicon", "download", "ui.favicon").on_click(
                    cx.listener(|this, _, window, cx| {
                        this.editor.update(cx, |editor, cx| {
                            editor.binary(taypeer_services::BinaryEdit::Icon(
                                taypeer_services::IconInput::Favicon(None),
                            ));
                            cx.notify();
                        });
                        let _ = window;
                    }),
                ));
            }
            if !matches!(entry_field, EntryField::Password | EntryField::Title) {
                let field = *entry_field;
                control = control.child(
                    icon_button(("clear-field", field as usize), "x", "ui.clear_value")
                        .disabled(!self.editor.read(cx).editable())
                        .on_click(cx.listener(move |this, _, _, cx| {
                            this.editor.update(cx, |editor, cx| {
                                editor.clear_field(field);
                                cx.notify();
                            });
                        })),
                );
            }
            result = result.child(input_row(
                entry_field.key(),
                input.focus_handle(cx),
                control,
                cx,
            ));
        }
        result
            .child(input_row(
                "notes",
                self.notes.focus_handle(cx),
                Textarea::new(&self.notes)
                    .disabled(!self.editor.read(cx).editable())
                    .aria_label(tr("notes"))
                    .appearance(false)
                    .bordered(false)
                    .h(rems(7.)),
                cx,
            ))
            .into_any_element()
    }
    fn advanced(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = self.editor.read(cx).content();
        let mut result = v_flex().child(
            h_flex()
                .justify_between()
                .pr_3()
                .child(section("attributes"))
                .child(
                    icon_button("add-attribute", "plus", "add_attribute")
                        .disabled(self.editor.read(cx).busy() || !self.editor.read(cx).editable())
                        .on_click(cx.listener(|this, _, window, cx| {
                            forms::attribute(this.editor.clone(), None, window, cx)
                        })),
                ),
        );
        for (index, attribute) in content.attributes.iter().enumerate() {
            let value = if attribute.protected {
                if attribute.value.is_empty() {
                    "••••••••••••".into()
                } else {
                    attribute.value.clone()
                }
            } else {
                attribute.value.clone()
            };
            let editor = self.editor.clone();
            result = result.child(
                h_flex()
                    .min_h(rems(2.75))
                    .px_6()
                    .gap_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(div().w(rems(9.)).truncate().child(attribute.key.clone()))
                    .child(div().flex_1().min_w_0().truncate().child(value))
                    .when(attribute.protected, |row| {
                        let id = attribute.id.clone();
                        row.child(
                            icon_button(("reveal-attribute", index), "eye", "ui.reveal_current")
                                .disabled(self.editor.read(cx).busy())
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.editor.update(cx, |editor, cx| {
                                        editor.reveal(id.clone());
                                        cx.notify();
                                    });
                                })),
                        )
                    })
                    .child(
                        Checkbox::new(("protect", index))
                            .checked(attribute.protected)
                            .disabled(
                                self.editor.read(cx).busy() || !self.editor.read(cx).editable(),
                            )
                            .accessibility_label(tr("protected"))
                            .tooltip(tr("protected"))
                            .on_click(move |checked, _, cx| {
                                editor.update(cx, |editor, cx| {
                                    editor.edit(|content| {
                                        if let Some(attribute) = content.attributes.get_mut(index) {
                                            attribute.protected = *checked;
                                        }
                                    });
                                    cx.notify();
                                })
                            }),
                    )
                    .child(
                        icon_button(("edit-attribute", index), "pencil", "ui.edit_attribute")
                            .on_click(cx.listener(move |this, _, window, cx| {
                                forms::attribute(this.editor.clone(), Some(index), window, cx)
                            })),
                    )
                    .child(
                        icon_button(("remove-attribute", index), "x", "remove")
                            .disabled(
                                self.editor.read(cx).busy() || !self.editor.read(cx).editable(),
                            )
                            .on_click(cx.listener(move |this, _, _, cx| {
                                this.editor.update(cx, |editor, cx| {
                                    editor.edit(|content| {
                                        content.attributes.remove(index);
                                    });
                                    cx.notify();
                                })
                            })),
                    ),
            );
        }
        result = result.child(
            h_flex()
                .justify_between()
                .pr_3()
                .child(section("ui.attachments"))
                .child(
                    icon_button("add-attachment", "file-plus-2", "ui.add_attachment").on_click(
                        cx.listener(|this, _, window, cx| {
                            forms::attachment(this.editor.clone(), None, window, cx)
                        }),
                    ),
                ),
        );
        for (index, attachment) in content.attachments.iter().enumerate() {
            result = result.child(
                h_flex()
                    .min_h(rems(2.75))
                    .px_6()
                    .gap_3()
                    .border_b_1()
                    .border_color(cx.theme().border)
                    .child(icon("file"))
                    .child(
                        div()
                            .flex_1()
                            .min_w_0()
                            .truncate()
                            .child(attachment.name.clone()),
                    )
                    .child(div().text_xs().child(attachment.bytes.map_or_else(
                        || tr("ui.attachment_unavailable").to_string(),
                        |bytes| format!("{} KiB", bytes / 1024),
                    )))
                    .child(
                        icon_button(
                            ("rename-attachment", index),
                            "pencil",
                            "ui.rename_attachment",
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                forms::attachment(this.editor.clone(), Some(index), window, cx)
                            },
                        )),
                    )
                    .child(
                        icon_button(("remove-attachment", index), "x", "remove").on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.editor.update(cx, |editor, cx| {
                                    editor.edit(|content| {
                                        content.attachments.remove(index);
                                    });
                                    cx.notify();
                                })
                            }),
                        ),
                    ),
            );
        }
        result
            .child(
                div()
                    .p_6()
                    .text_xs()
                    .text_color(cx.theme().muted_foreground)
                    .child(tr("ui.attachments_hint")),
            )
            .into_any_element()
    }
    fn color_picker(
        editor: &Entity<EditorStore>,
        background: bool,
        window: &mut Window,
        cx: &mut Context<Self>,
        subscriptions: &mut Vec<Subscription>,
    ) -> Entity<color_picker::ColorPickerState> {
        let initial = if background {
            editor.read(cx).content().background
        } else {
            editor.read(cx).content().foreground
        };
        let state = cx.new(|cx| {
            let state = color_picker::ColorPickerState::new(window, cx);
            if let Some(color) = initial {
                state.default_value(rgba(color))
            } else {
                state
            }
        });
        let editor = editor.clone();
        subscriptions.push(cx.subscribe(
            &state,
            move |_, _, event: &color_picker::ColorPickerEvent, cx| {
                let color_picker::ColorPickerEvent::Change(color) = event;
                if !editor.read(cx).editable() {
                    return;
                }
                let value = color.map(|c| {
                    let c = c.to_rgb();
                    u32::from_be_bytes(
                        [c.r, c.g, c.b, c.a].map(|v| (v.clamp(0., 1.) * 255.).round() as u8),
                    )
                });
                editor.update(cx, |editor, cx| {
                    editor.edit(|content| {
                        if background {
                            content.background = value;
                        } else {
                            content.foreground = value;
                        }
                    });
                    cx.notify();
                });
            },
        ));
        state
    }

    fn color_control(&self, background: bool, cx: &Context<Self>) -> AnyElement {
        let state = if background {
            &self.background
        } else {
            &self.foreground
        };
        let picker = state.clone();
        let editor = self.editor.clone();
        let value = if background {
            self.editor.read(cx).content().background
        } else {
            self.editor.read(cx).content().foreground
        };
        h_flex()
            .gap_2()
            .child(
                color_picker::ColorPicker::new(state)
                    .label(color_text(value))
                    .accessibility_label(tr(if background {
                        "ui.background_color"
                    } else {
                        "ui.foreground_color"
                    })),
            )
            .when(value.is_some(), |el| {
                el.child(
                    icon_button(
                        if background {
                            "reset-background"
                        } else {
                            "reset-foreground"
                        },
                        "x",
                        "ui.clear_value",
                    )
                    .disabled(!self.editor.read(cx).editable())
                    .on_click(move |_, window, cx| {
                        picker.update(cx, |picker, cx| picker.clear_value(window, cx));
                        editor.update(cx, |editor, cx| {
                            editor.edit(|content| {
                                if background {
                                    content.background = None;
                                } else {
                                    content.foreground = None;
                                }
                            });
                            cx.notify();
                        });
                    }),
                )
            })
            .into_any_element()
    }
    fn appearance(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = self.editor.read(cx).content();
        v_flex()
            .child(row(
                "ui.icon",
                h_flex()
                    .gap_3()
                    .child(super::images::stored_icon(
                        &content.icon,
                        content.icon_blob.as_ref(),
                        cx,
                    ))
                    .child(
                        Button::new("choose-icon")
                            .label(tr("ui.choose_icon"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                super::icons::choose(
                                    this.editor.read(cx).content().icon.clone(),
                                    {
                                        let editor = this.editor.clone();
                                        move |name, cx| {
                                            editor.update(cx, |editor, cx| {
                                                editor.edit(|content| content.icon = name.into());
                                                cx.notify();
                                            })
                                        }
                                    },
                                    window,
                                    cx,
                                )
                            })),
                    )
                    .child(
                        Button::new("image-file")
                            .ghost()
                            .label(tr("ui.image_file"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                forms::image_file(this.editor.clone(), window, cx)
                            })),
                    )
                    .child(
                        Button::new("image-url")
                            .ghost()
                            .label(tr("ui.image_url"))
                            .on_click(cx.listener(|this, _, window, cx| {
                                forms::image_url(this.editor.clone(), window, cx)
                            })),
                    ),
                cx,
            ))
            .child(row(
                "ui.foreground_color",
                self.color_control(false, cx),
                cx,
            ))
            .child(row("ui.background_color", self.color_control(true, cx), cx))
            .child(section("ui.preview"))
            .child(
                div()
                    .mx_6()
                    .p_4()
                    .rounded_sm()
                    .bg(content
                        .background
                        .map(|c| rgba(c).into())
                        .unwrap_or(cx.theme().background))
                    .text_color(
                        content
                            .foreground
                            .map(|c| rgba(c).into())
                            .unwrap_or(cx.theme().foreground),
                    )
                    .child(
                        h_flex()
                            .gap_3()
                            .child(super::images::stored_icon(
                                &content.icon,
                                content.icon_blob.as_ref(),
                                cx,
                            ))
                            .child(content.title.clone()),
                    ),
            )
            .into_any_element()
    }
}
impl Render for EditorView {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.store.read(cx).state().tab;
        let content = match tab {
            EntryTab::Overview => self.overview(cx),
            EntryTab::Advanced => self.advanced(cx),
            EntryTab::Appearance => self.appearance(cx),
            EntryTab::Properties | EntryTab::History => div().into_any_element(),
        };
        v_flex()
            .child(content)
            .when_some(self.editor.read(cx).error(), |el, error| {
                el.child(
                    div()
                        .p_4()
                        .text_color(cx.theme().danger)
                        .child(tr(error.key())),
                )
            })
    }
}
