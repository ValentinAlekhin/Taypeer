//! Editable entry tabs. The draft owns values; these widgets adapt native input.

use super::{forms, style::*, workspace::WorkspaceStore};
use crate::actions::ToggleEditorPassword;
use crate::ui_state::*;
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::{
    component::{
        button::*,
        checkbox::Checkbox,
        input::{InputEvent, InputState, TextareaState},
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
pub(super) fn field_row(
    field: EntryField,
    input: &Entity<InputState>,
    control: impl IntoElement,
    actions: impl IntoIterator<Item = AnyElement>,
    cx: &App,
) -> AnyElement {
    super::card::overview_row(
        field,
        input_row(
            field.key(),
            input.focus_handle(cx),
            input_control(
                field.key(),
                super::clipboard::secret_field(control, input, field == EntryField::Password),
            )
            .children(actions),
            cx,
        ),
        cx,
    )
}
pub(super) fn notes_row(
    notes: &Entity<TextareaState>,
    disabled: bool,
    window: &Window,
    cx: &App,
) -> AnyElement {
    super::card::overview_row(
        EntryField::Notes,
        input_row(
            "notes",
            notes.focus_handle(cx),
            div().id("field-notes").test_support().child(focus_control(
                notes.focus_handle(cx),
                super::row_textarea::control(notes, "notes", false, disabled),
                window,
                cx,
            )),
            cx,
        ),
        cx,
    )
}
impl EditorView {
    pub fn new(
        store: Entity<WorkspaceStore>,
        editor: Entity<EditorStore>,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) -> Self {
        let fields: Vec<_> = EntryField::ALL
            .into_iter()
            .filter(|f| *f != EntryField::Notes)
            .map(|f| {
                (f, {
                    let value = f.value(editor.read(cx).content()).to_owned();
                    input(&value, f == EntryField::Password, window, cx)
                })
            })
            .collect();
        if editor.read(cx).content().has_password
            && let Some((_, password)) = fields
                .iter()
                .find(|(field, _)| *field == EntryField::Password)
        {
            password.update(cx, |input, cx| {
                input.set_placeholder("••••••••••", window, cx)
            });
        }
        let initial_notes = editor.read(cx).content().notes.clone();
        let notes = cx.new(|cx| {
            let mut state = TextareaState::new(window, cx).auto_grow(1, 20);
            state.set_value(initial_notes, window, cx);
            state
        });
        let mut subscriptions = vec![
            cx.observe_in(&editor, window, |this, editor, window, cx| {
                for (field, input) in &this.fields {
                    let changed =
                        input.read(cx).value().as_str() != field.value(editor.read(cx).content());
                    if changed {
                        let value = field.value(editor.read(cx).content()).to_owned();
                        input.update(cx, |input, cx| input.set_value(value, window, cx));
                    }
                    if *field == EntryField::Password {
                        let placeholder = if editor.read(cx).content().has_password {
                            "••••••••••"
                        } else {
                            ""
                        };
                        if input.read(cx).presentation().placeholder().as_str() != placeholder {
                            input.update(cx, |input, cx| {
                                input.set_placeholder(placeholder, window, cx)
                            });
                        }
                    }
                }
                if this.notes.read(cx).value().as_str() != editor.read(cx).content().notes {
                    let value = editor.read(cx).content().notes.clone();
                    this.notes
                        .update(cx, |input, cx| input.set_value(value, window, cx));
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
                            editor.set_field(field, value);
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
                        editor.set_field(EntryField::Notes, value);
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
    fn overview(&self, window: &Window, cx: &mut Context<Self>) -> AnyElement {
        let mut result = v_flex();
        for (entry_field, input) in &self.fields {
            let field_control = super::style::entry_field(input, entry_field.key())
                .disabled(!self.editor.read(cx).editable())
                .when(*entry_field == EntryField::Password, |field| {
                    field.suffix(
                        icon_button(
                            "editor-password-eye",
                            if input.read(cx).presentation().is_masked() {
                                "eye"
                            } else {
                                "eye-off"
                            },
                            if input.read(cx).presentation().is_masked() {
                                "show"
                            } else {
                                "hide"
                            },
                        )
                        .disabled(!self.editor.read(cx).editable() || self.editor.read(cx).busy())
                        .on_click(
                            cx.listener(|this, _, window, cx| this.toggle_password(window, cx)),
                        ),
                    )
                });
            let mut actions = Vec::new();
            if *entry_field == EntryField::Password {
                actions.push(
                    icon_button("generator", "dice-5", "ui.generator")
                        .on_click(cx.listener(|this, _, window, cx| {
                            super::inspector::open_generator(this.editor.clone(), window, cx)
                        }))
                        .into_any_element(),
                );
            }
            if *entry_field == EntryField::Url {
                actions.push(
                    icon_button("favicon", "download", "ui.favicon")
                        .on_click(cx.listener(|this, _, window, cx| {
                            this.editor.update(cx, |editor, cx| {
                                editor.binary(taypeer_services::BinaryEdit::Icon(
                                    taypeer_services::IconInput::Favicon(None),
                                ));
                                cx.notify();
                            });
                            let _ = window;
                        }))
                        .into_any_element(),
                );
            }
            result = result.child(field_row(*entry_field, input, field_control, actions, cx));
        }
        result
            .child(notes_row(
                &self.notes,
                !self.editor.read(cx).editable(),
                window,
                cx,
            ))
            .into_any_element()
    }
    pub(super) fn toggle_password(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if !self.editor.read(cx).editable() || self.editor.read(cx).busy() {
            return;
        }
        let Some((_, input)) = self
            .fields
            .iter()
            .find(|(field, _)| *field == EntryField::Password)
        else {
            return;
        };
        let masked = input.read(cx).presentation().is_masked();
        // An untouched password is represented by Keep and a masked placeholder.
        // Reveal it only after this explicit command; typed text remains authoritative.
        if masked
            && input.read(cx).value().is_empty()
            && self.editor.read(cx).content().has_password
        {
            self.editor.update(cx, |editor, cx| {
                editor.reveal(None);
                cx.notify();
            });
        }
        input.update(cx, |input, cx| input.set_masked(!masked, window, cx));
        cx.notify();
    }
    pub(super) fn password_masked(&self, cx: &App) -> bool {
        self.fields
            .iter()
            .find(|(field, _)| *field == EntryField::Password)
            .is_none_or(|(_, input)| input.read(cx).presentation().is_masked())
    }
    fn advanced(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = self.editor.read(cx).content();
        let mut result = v_flex().child(super::details::section_header(
            "attributes",
            Some(
                icon_button("add-attribute", "plus", "add_attribute")
                    .disabled(self.editor.read(cx).busy() || !self.editor.read(cx).editable())
                    .on_click(cx.listener(|this, _, window, cx| {
                        forms::attribute(this.editor.clone(), None, window, cx)
                    }))
                    .into_any_element(),
            ),
        ));
        for (index, attribute) in content.attributes.iter().enumerate() {
            let protect_id = attribute.id.clone();
            let edit_id = attribute.id.clone();
            let remove_id = attribute.id.clone();
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
            result = result.child(super::details::attribute_row(
                &attribute.key,
                value,
                Some(
                    h_flex()
                        .gap_2()
                        .when(attribute.protected, |row| {
                            let id = attribute.id.clone();
                            row.child(
                                icon_button(
                                    ("reveal-attribute", index),
                                    "eye",
                                    "ui.reveal_current",
                                )
                                .disabled(self.editor.read(cx).busy())
                                .on_click(cx.listener(
                                    move |this, _, _, cx| {
                                        this.editor.update(cx, |editor, cx| {
                                            editor.reveal(id.clone());
                                            cx.notify();
                                        });
                                    },
                                )),
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
                                        if let Some(id) = &protect_id {
                                            editor.set_attribute_protected(id, *checked);
                                        }
                                        cx.notify();
                                    })
                                }),
                        )
                        .child(
                            icon_button(("edit-attribute", index), "pencil", "ui.edit_attribute")
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    forms::attribute(
                                        this.editor.clone(),
                                        edit_id.clone(),
                                        window,
                                        cx,
                                    )
                                })),
                        )
                        .child(
                            icon_button(("remove-attribute", index), "x", "remove")
                                .disabled(
                                    self.editor.read(cx).busy() || !self.editor.read(cx).editable(),
                                )
                                .on_click(cx.listener(move |this, _, _, cx| {
                                    this.editor.update(cx, |editor, cx| {
                                        if let Some(id) = &remove_id {
                                            editor.remove_attribute(id);
                                        }
                                        cx.notify();
                                    })
                                })),
                        )
                        .into_any_element(),
                ),
                cx,
            ));
        }
        result = result.child(super::details::section_header(
            "ui.attachments",
            Some(
                icon_button("add-attachment", "file-plus-2", "ui.add_attachment")
                    .on_click(cx.listener(|this, _, window, cx| {
                        forms::attachment(this.editor.clone(), None, window, cx)
                    }))
                    .into_any_element(),
            ),
        ));
        for (index, attachment) in content.attachments.iter().enumerate() {
            let rename_id = attachment.id.clone();
            let remove_id = attachment.id.clone();
            result = result.child(super::details::attachment_row(
                &attachment.name,
                Some(attachment.bytes.map_or_else(
                    || tr("ui.attachment_unavailable").to_string(),
                    |bytes| format!("{} KiB", bytes / 1024),
                )),
                h_flex()
                    .gap_2()
                    .child(
                        icon_button(
                            ("rename-attachment", index),
                            "pencil",
                            "ui.rename_attachment",
                        )
                        .on_click(cx.listener(
                            move |this, _, window, cx| {
                                forms::attachment(
                                    this.editor.clone(),
                                    Some(rename_id.clone()),
                                    window,
                                    cx,
                                )
                            },
                        )),
                    )
                    .child(
                        icon_button(("remove-attachment", index), "x", "remove").on_click(
                            cx.listener(move |this, _, _, cx| {
                                this.editor.update(cx, |editor, cx| {
                                    editor.remove_attachment(&remove_id);
                                    cx.notify();
                                })
                            }),
                        ),
                    ),
                cx,
            ));
        }
        result
            .child(super::details::attachments_hint(cx))
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
                    editor.set_color(background, value);
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
        super::details::color_control(
            state,
            background,
            value,
            value.map(|_| {
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
                        editor.set_color(background, None);
                        cx.notify();
                    });
                })
                .into_any_element()
            }),
        )
    }
    fn appearance(&self, cx: &mut Context<Self>) -> AnyElement {
        let content = self.editor.read(cx).content();
        super::details::appearance(
            content,
            super::details::icon_control(
                content,
                [
                    super::details::choose_icon(Button::new("choose-icon"))
                        .on_click(cx.listener(|this, _, window, cx| {
                            super::icons::choose(
                                this.editor.read(cx).content().icon.clone(),
                                {
                                    let editor = this.editor.clone();
                                    move |name, cx| {
                                        editor.update(cx, |editor, cx| {
                                            editor.set_icon(name.into());
                                            cx.notify();
                                        })
                                    }
                                },
                                window,
                                cx,
                            )
                        }))
                        .into_any_element(),
                    icon_button("image-file", "image", "ui.image_file")
                        .on_click(cx.listener(|this, _, window, cx| {
                            forms::image_file(this.editor.clone(), window, cx)
                        }))
                        .into_any_element(),
                    icon_button("image-url", "link", "ui.image_url")
                        .on_click(cx.listener(|this, _, window, cx| {
                            forms::image_url(this.editor.clone(), window, cx)
                        }))
                        .into_any_element(),
                ],
                cx,
            ),
            self.color_control(false, cx),
            self.color_control(true, cx),
            cx,
        )
    }
}
impl Render for EditorView {
    fn render(&mut self, window: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        let tab = self.store.read(cx).state().tab;
        let content = match tab {
            EntryTab::Overview => self.overview(window, cx),
            EntryTab::Advanced => self.advanced(cx),
            EntryTab::Appearance => self.appearance(cx),
            EntryTab::Properties | EntryTab::History => div().into_any_element(),
        };
        v_flex()
            .bg(cx.theme().background)
            .child(content)
            .on_action(cx.listener(|this, _: &ToggleEditorPassword, window, cx| {
                this.toggle_password(window, cx);
                cx.stop_propagation();
            }))
            .when_some(self.editor.read(cx).error(), |el, error| {
                el.child(
                    div()
                        .id("autosave-error")
                        .test_support()
                        .aria_label(tr(error.key()))
                        .p_4()
                        .text_color(cx.theme().danger)
                        .child(tr(error.key())),
                )
            })
    }
}
