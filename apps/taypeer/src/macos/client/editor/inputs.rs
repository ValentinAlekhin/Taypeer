use super::{Client, Editor, EntryField};
use crate::macos::common::{format_date, input, tr};
use gpui_kit::component::input::{InputEvent, InputState, TextareaState};
use gpui_kit::*;
use taypeer_services::{DraftView, SessionToken};

pub(super) struct AttributeInputs {
    pub(super) name: Entity<InputState>,
    pub(super) value: Entity<InputState>,
}
pub(super) struct EditorInputs {
    pub(super) title: Entity<InputState>,
    pub(super) username: Entity<InputState>,
    pub(super) password: Entity<InputState>,
    pub(super) url: Entity<InputState>,
    pub(super) notes: Entity<TextareaState>,
    pub(super) tags: Entity<TextareaState>,
    pub(super) expiry: Entity<InputState>,
}
impl EditorInputs {
    fn new(draft: &DraftView, window: &mut Window, cx: &mut App) -> Self {
        let fields = &draft.fields;
        let multiline = |value: &str, window: &mut Window, cx: &mut App| {
            cx.new(|cx| {
                let mut state = TextareaState::new(window, cx);
                state.set_value(value, window, cx);
                state
            })
        };
        let inputs = Self {
            title: input(&fields.title, false, window, cx),
            username: input(
                fields.username.as_deref().unwrap_or_default(),
                false,
                window,
                cx,
            ),
            password: input(
                fields.password.as_deref().unwrap_or_default(),
                true,
                window,
                cx,
            ),
            url: input(fields.url.as_deref().unwrap_or_default(), false, window, cx),
            notes: multiline(fields.notes.as_deref().unwrap_or_default(), window, cx),
            tags: multiline(&fields.tags.join("\n"), window, cx),
            expiry: input(
                &draft
                    .expiry_input
                    .clone()
                    .unwrap_or_else(|| fields.expires_at.map(format_date).unwrap_or_default()),
                false,
                window,
                cx,
            ),
        };
        for (state, absent) in [
            (&inputs.username, fields.username.is_none()),
            (&inputs.password, fields.password.is_none()),
            (&inputs.url, fields.url.is_none()),
        ] {
            if absent {
                state.update(cx, |state, cx| {
                    state.set_placeholder(tr("absent"), window, cx)
                });
            }
        }
        inputs.expiry.update(cx, |state, cx| {
            state.set_placeholder("YYYY-MM-DD HH:MM", window, cx)
        });
        inputs
    }
    pub(super) fn single(&self, field: EntryField) -> Option<&Entity<InputState>> {
        match field {
            EntryField::Title => Some(&self.title),
            EntryField::Username => Some(&self.username),
            EntryField::Password => Some(&self.password),
            EntryField::Url => Some(&self.url),
            EntryField::Expiry => Some(&self.expiry),
            _ => None,
        }
    }
    pub(super) fn multiline(&self, field: EntryField) -> Option<&Entity<TextareaState>> {
        match field {
            EntryField::Notes => Some(&self.notes),
            EntryField::Tags => Some(&self.tags),
            _ => None,
        }
    }
    pub(super) fn focus_title(&self, window: &mut Window, cx: &mut App) {
        self.title.update(cx, |state, cx| state.focus(window, cx));
    }
    pub(super) fn clear(&self, field: EntryField, window: &mut Window, cx: &mut App) {
        if let Some(state) = self.single(field) {
            state.update(cx, |state, cx| state.set_value("", window, cx));
        }
        if let Some(state) = self.multiline(field) {
            state.update(cx, |state, cx| state.set_value("", window, cx));
        }
    }
}
impl Editor {
    pub(super) fn new(
        session: SessionToken,
        draft: DraftView,
        window: &mut Window,
        cx: &mut Context<Client>,
    ) -> Self {
        let inputs = EditorInputs::new(&draft, window, cx);
        let mut field_subscriptions = Vec::new();
        for field in EntryField::ALL {
            let token = session.clone();
            if let Some(state) = inputs.single(field) {
                field_subscriptions.push(cx.subscribe_in(
                    state,
                    window,
                    move |this, state, event: &InputEvent, _, cx| {
                        let current = this
                            .editor
                            .as_ref()
                            .and_then(|editor| editor.inputs.single(field))
                            .is_some_and(|input| input.entity_id() == state.entity_id());
                        if matches!(event, InputEvent::Change) && this.accepts(&token) && current {
                            this.change_field(field, state.read(cx).value().to_string(), cx);
                        }
                    },
                ));
            } else if let Some(state) = inputs.multiline(field) {
                field_subscriptions.push(cx.subscribe_in(
                    state,
                    window,
                    move |this, state, event: &InputEvent, _, cx| {
                        let current = this
                            .editor
                            .as_ref()
                            .and_then(|editor| editor.inputs.multiline(field))
                            .is_some_and(|input| input.entity_id() == state.entity_id());
                        if matches!(event, InputEvent::Change) && this.accepts(&token) && current {
                            this.change_field(field, state.read(cx).value().to_string(), cx);
                        }
                    },
                ));
            }
        }
        Self {
            session,
            draft,
            inputs,
            attributes: Vec::new(),
            _field_subscriptions: field_subscriptions,
            attribute_subscriptions: Vec::new(),
        }
    }
}
impl Client {
    pub(super) fn install_attributes(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let Some(editor) = &mut self.editor else {
            return;
        };
        editor.attribute_subscriptions.clear();
        editor.attributes.clear();
        for (index, attribute) in editor.draft.fields.attributes.iter().enumerate() {
            let name = input(&attribute.name, false, window, cx);
            let value = input(&attribute.value, attribute.protected, window, cx);
            for (is_name, state) in [(true, &name), (false, &value)] {
                let token = editor.session.clone();
                editor.attribute_subscriptions.push(cx.subscribe_in(
                    state,
                    window,
                    move |this, state, event: &InputEvent, _, cx| {
                        let current = this
                            .editor
                            .as_ref()
                            .and_then(|editor| editor.attributes.get(index))
                            .is_some_and(|attribute| {
                                if is_name {
                                    attribute.name.entity_id() == state.entity_id()
                                } else {
                                    attribute.value.entity_id() == state.entity_id()
                                }
                            });
                        if matches!(event, InputEvent::Change) && this.accepts(&token) && current {
                            if let Some(editor) = &mut this.editor
                                && let Some(attribute) =
                                    editor.draft.fields.attributes.get_mut(index)
                            {
                                if is_name {
                                    attribute.name = state.read(cx).value().to_string();
                                } else {
                                    attribute.value = state.read(cx).value().to_string();
                                }
                            }
                            this.push_draft(cx);
                        }
                    },
                ));
            }
            editor.attributes.push(AttributeInputs { name, value });
        }
    }
}
