use super::{Client, Editor, EntryField, EntryTab};
use crate::macos::common::{field_row, tr};
use gpui_kit::component::{
    button::*,
    input::{Input, InputContentType, Textarea},
    *,
};
use gpui_kit::prelude::FluentBuilder;
use gpui_kit::*;

impl Editor {
    fn field_control(&self, field: EntryField) -> AnyElement {
        if let Some(state) = self.inputs.multiline(field) {
            Textarea::new(state)
                .aria_label(tr(field.key()))
                .h(rems(5.))
                .into_any_element()
        } else if let Some(state) = self.inputs.single(field) {
            Input::new(state)
                .aria_label(tr(field.key()))
                .when(field == EntryField::Password, |control| {
                    control
                        .mask_toggle()
                        .content_type(InputContentType::Password)
                })
                .into_any_element()
        } else {
            unreachable!("every EntryField has exactly one input")
        }
    }
}
impl Client {
    pub(in crate::macos::client) fn editor_content(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        if self.tab == EntryTab::Attributes {
            return self.attribute_editor(cx);
        }
        v_flex()
            .children(EntryField::ALL.into_iter().map(|field| {
                let value = v_flex().gap_1().child(editor.field_control(field)).when(
                    field.optional(),
                    |row| {
                        row.child(
                            Button::new(SharedString::from(format!("unset-{}", field.key())))
                                .label(tr("clear"))
                                .on_click(cx.listener(move |this, _, window, cx| {
                                    this.clear_optional(field, window, cx)
                                })),
                        )
                    },
                );
                field_row(field.key(), value, cx)
            }))
            .into_any_element()
    }
    fn attribute_editor(&self, cx: &mut Context<Self>) -> AnyElement {
        let Some(editor) = &self.editor else {
            return div().into_any_element();
        };
        v_flex()
            .gap_3()
            .children(
                editor
                    .attributes
                    .iter()
                    .enumerate()
                    .map(|(index, attribute)| {
                        let protected = editor.draft.fields.attributes[index].protected;
                        v_flex()
                            .gap_2()
                            .border_b_1()
                            .border_color(cx.theme().border)
                            .pb_3()
                            .child(Input::new(&attribute.name).aria_label(tr("name")))
                            .child(Input::new(&attribute.value).aria_label(tr("value")).when(
                                protected,
                                |control| {
                                    control
                                        .mask_toggle()
                                        .content_type(InputContentType::Password)
                                },
                            ))
                            .child(
                                h_flex()
                                    .gap_2()
                                    .child(
                                        Button::new(("protect-attribute", index))
                                            .label(tr("protected"))
                                            .selected(protected)
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.toggle_attribute(index, window, cx)
                                            })),
                                    )
                                    .child(
                                        Button::new(("remove-attribute", index))
                                            .label(tr("remove"))
                                            .on_click(cx.listener(move |this, _, window, cx| {
                                                this.remove_attribute(index, window, cx)
                                            })),
                                    ),
                            )
                            .into_any_element()
                    }),
            )
            .child(
                Button::new("add-attribute")
                    .label(tr("add_attribute"))
                    .on_click(cx.listener(|this, _, window, cx| this.add_attribute(window, cx))),
            )
            .into_any_element()
    }
}
