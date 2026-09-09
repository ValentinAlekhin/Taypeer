mod fields;
mod inputs;
mod view;

use super::{Client, EntryTab};
use fields::EntryField;
use gpui_kit::*;
use inputs::{AttributeInputs, EditorInputs};
use taypeer_services::{DraftView, EditableAttribute, SessionToken};

pub(super) struct Editor {
    pub(super) session: SessionToken,
    pub(super) draft: DraftView,
    inputs: EditorInputs,
    attributes: Vec<AttributeInputs>,
    // Owning these subscriptions ties callbacks to the editor lifetime.
    _field_subscriptions: Vec<Subscription>,
    attribute_subscriptions: Vec<Subscription>,
}

impl Client {
    pub(super) fn edit(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let (Some(token), Some(id)) = (&self.session, &self.selected) {
            match self.service.start_edit_entry(token, id) {
                Ok(reply) if self.accepts(&reply.session) => {
                    self.install_editor(reply.session, reply.value, window, cx)
                }
                _ => self.error = Some("error"),
            }
        }
        cx.notify();
    }
    pub(super) fn install_editor(
        &mut self,
        session: SessionToken,
        draft: DraftView,
        window: &mut Window,
        cx: &mut Context<Self>,
    ) {
        self.tab = EntryTab::Overview;
        self.editor = Some(Editor::new(session, draft, window, cx));
        self.install_attributes(window, cx);
        if let Some(editor) = &self.editor {
            editor.inputs.focus_title(window, cx);
        }
    }
    fn change_field(&mut self, field: EntryField, value: String, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(editor) = &mut self.editor else {
            return;
        };
        if field
            .apply(&mut editor.draft.fields, value.clone())
            .is_err()
        {
            self.error = Some("invalid_expiry");
            match self
                .service
                .set_draft_expiry_input(&editor.session, Some(value))
            {
                Ok(reply)
                    if self.session.as_ref() == Some(&reply.session)
                        && self.service.is_current(&reply.session) =>
                {
                    editor.draft = reply.value
                }
                _ => self.error = Some("error"),
            }
            cx.notify();
            return;
        }
        if field == EntryField::Expiry
            && self
                .service
                .set_draft_expiry_input(&editor.session, None)
                .is_err()
        {
            self.error = Some("error");
            cx.notify();
            return;
        }
        self.error = None;
        self.push_draft(cx);
    }
    fn push_draft(&mut self, cx: &mut Context<Self>) {
        if self.busy {
            return;
        }
        let Some(editor) = &mut self.editor else {
            return;
        };
        match self
            .service
            .update_draft(&editor.session, editor.draft.fields.clone())
        {
            Ok(reply)
                if self.session.as_ref() == Some(&reply.session)
                    && self.service.is_current(&reply.session) =>
            {
                editor.draft = reply.value
            }
            _ => self.error = Some("error"),
        }
        cx.notify();
    }
    pub(super) fn save(&mut self, window: &mut Window, cx: &mut Context<Self>) -> bool {
        if self.busy {
            return false;
        }
        let Some(editor) = &self.editor else {
            return true;
        };
        if editor.draft.expiry_input.is_some() {
            self.error = Some("invalid_expiry");
            cx.notify();
            return false;
        }
        let token = editor.session.clone();
        self.run_io(
            window,
            cx,
            move |service| service.save_draft(&token),
            |this, result, window, cx| match result {
                Ok(reply) if this.accepts(&reply.session) => {
                    this.selected = Some(reply.value);
                    this.editor = None;
                    this.root_focus.focus(window, cx);
                    this.revealed.clear();
                    this.error = None;
                    if let Some(action) = this.pending.take() {
                        this.perform_navigation(action, window, cx);
                    }
                }
                Err(error) => this.error = Some(super::files::file_error(error)),
                _ => this.error = Some("error"),
            },
        );
        false
    }
    fn clear_optional(&mut self, field: EntryField, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editor {
            editor.inputs.clear(field, window, cx);
            field.unset(&mut editor.draft.fields);
        }
        self.push_draft(cx);
    }
    fn add_attribute(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editor {
            editor.draft.fields.attributes.push(EditableAttribute {
                id: None,
                name: String::new(),
                value: String::new(),
                protected: true,
            });
        }
        self.push_draft(cx);
        self.install_attributes(window, cx);
    }
    fn remove_attribute(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editor
            && index < editor.draft.fields.attributes.len()
        {
            editor.draft.fields.attributes.remove(index);
        }
        self.push_draft(cx);
        self.install_attributes(window, cx);
    }
    fn toggle_attribute(&mut self, index: usize, window: &mut Window, cx: &mut Context<Self>) {
        if let Some(editor) = &mut self.editor
            && let Some(attribute) = editor.draft.fields.attributes.get_mut(index)
        {
            attribute.protected = !attribute.protected;
            editor.attributes[index].value.update(cx, |state, cx| {
                state.set_masked(attribute.protected, window, cx)
            });
        }
        self.push_draft(cx);
    }
}
