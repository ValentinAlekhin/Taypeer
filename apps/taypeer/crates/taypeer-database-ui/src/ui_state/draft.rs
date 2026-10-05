//! Editable widget values and ordered delivery to the service-owned draft.
use super::*;
use crate::backend::{Connection, Ticket};
use std::time::{Duration, Instant};
use taypeer_runtime::Command;
use taypeer_services::{
    BinaryEdit, BinaryRequest, BinaryTarget, EditorView, EntryPatch, FieldUpdate,
};
use zeroize::Zeroize;

mod autosave;
pub(crate) use autosave::SaveEvent;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum EntryField {
    Title,
    Username,
    Password,
    Url,
    Tags,
    Notes,
    Expires,
}
impl EntryField {
    pub const ALL: [Self; 7] = [
        Self::Title,
        Self::Username,
        Self::Password,
        Self::Url,
        Self::Tags,
        Self::Expires,
        Self::Notes,
    ];
    pub fn key(self) -> &'static str {
        match self {
            Self::Title => "name",
            Self::Username => "username",
            Self::Password => "password",
            Self::Url => "url",
            Self::Tags => "ui.tags",
            Self::Notes => "notes",
            Self::Expires => "ui.expires",
        }
    }
    pub fn value(self, content: &EntryContent) -> &str {
        match self {
            Self::Title => &content.title,
            Self::Username => &content.username,
            Self::Password => &content.password,
            Self::Url => &content.url,
            Self::Tags => &content.tags,
            Self::Notes => &content.notes,
            Self::Expires => &content.expires,
        }
    }
    pub fn set(self, content: &mut EntryContent, value: String) {
        *match self {
            Self::Title => &mut content.title,
            Self::Username => &mut content.username,
            Self::Password => &mut content.password,
            Self::Url => &mut content.url,
            Self::Tags => &mut content.tags,
            Self::Notes => &mut content.notes,
            Self::Expires => &mut content.expires,
        } = value;
    }
}
pub(crate) struct EditorStore {
    connection: Connection,
    identity: taypeer_services::DraftIdentity,
    last_input: Instant,
    durable_revision: Option<u64>,
    snapshot: Option<autosave::SaveAttempt>,
    save_event: Option<SaveEvent>,
    content: EntryContent,
    dirty: bool,
    preview: Option<taypeer_services::IconPreview>,
    error: Option<FormError>,
    save_error: Option<FormError>,
    pending: Vec<Ticket<()>>,
    projection: Option<(u64, Ticket<EditorView>)>,
    revision: u64,
    forms_pending: usize,
    needs_projection: bool,
    reveal: Option<(
        u64,
        Option<taypeer_core::AttributeId>,
        Ticket<zeroize::Zeroizing<String>>,
    )>,
}
impl EditorStore {
    pub fn from_view(connection: Connection, view: EditorView) -> Self {
        let mut result = Self {
            connection,
            identity: view.identity.clone(),
            last_input: Instant::now(),
            durable_revision: (!view.dirty).then_some(0),
            snapshot: None,
            save_event: None,
            content: EntryContent::default(),
            dirty: view.dirty,
            preview: None,
            error: None,
            save_error: None,
            pending: Vec::new(),
            projection: None,
            revision: 0,
            forms_pending: 0,
            needs_projection: false,
            reveal: None,
        };
        result.install(view);
        result
    }
    fn install(&mut self, view: EditorView) {
        self.identity = view.identity.clone();
        self.dirty = view.dirty;
        self.content = EntryContent {
            title: view.fields.title,
            username: view.fields.username.unwrap_or_default(),
            password: std::mem::take(&mut self.content.password),
            has_password: view.has_password,
            url: view.fields.url.unwrap_or_default(),
            tags: view.fields.tags.join("\n"),
            notes: view.fields.notes.unwrap_or_default(),
            expires: view
                .expiry_input
                .unwrap_or_else(|| view.fields.expires_at.map(format_date).unwrap_or_default()),
            attributes: view
                .fields
                .attributes
                .into_iter()
                .map(|a| Attribute {
                    has_value: !a.value.is_empty(),
                    id: a.id,
                    key: a.name,
                    value: a.value,
                    protected: a.protected,
                })
                .collect(),
            attachments: view
                .attachments
                .into_iter()
                .map(|a| Attachment {
                    id: a.id,
                    blob: a.blob,
                    name: a.name,
                    bytes: None,
                })
                .collect(),
            icon: icon_name(&view.appearance.icon),
            icon_blob: view.appearance.icon.blob().cloned(),
            foreground: color_value(view.appearance.foreground),
            background: color_value(view.appearance.background),
        };
        super::catalog::set_attachment_sizes(&mut self.content, &view.binary);
        self.preview = view.icon_preview;
    }
    pub fn take_preview(&mut self) -> Option<taypeer_services::IconPreview> {
        self.preview.take()
    }
    pub fn database(&self) -> &DatabaseId {
        &self.connection.database
    }
    pub fn content(&self) -> &EntryContent {
        &self.content
    }
    pub fn editable(&self) -> bool {
        self.connection.control.is_open()
    }
    pub fn save_blocked(&self) -> bool {
        self.error.is_some()
    }
    pub fn unacknowledged(&self) -> bool {
        !self.pending.is_empty() || self.forms_pending > 0
    }
    pub fn status_key(&self) -> &'static str {
        if !self.durably_current() {
            "ui.autosave_pending"
        } else if self.dirty {
            "ui.draft_saved"
        } else if self.revision == 0
            && matches!(
                self.identity.target,
                taypeer_services::DraftTarget::NewEntry { .. }
            )
        {
            "ui.autosave_ready"
        } else {
            "ui.autosave_saved"
        }
    }
    pub fn error(&self) -> Option<FormError> {
        self.error.or(self.save_error)
    }
    pub fn busy(&self) -> bool {
        !self.pending.is_empty()
            || self.projection.is_some()
            || self.needs_projection
            || self.forms_pending > 0
    }
    pub fn connection(&self) -> &Connection {
        &self.connection
    }
    pub fn form_command(&mut self, command: Command) -> Ticket<()> {
        self.input_changed();
        self.forms_pending += 1;
        self.connection.command(command)
    }
    pub fn finish_form(&mut self, result: &crate::backend::Result<()>) {
        self.forms_pending = self.forms_pending.saturating_sub(1);
        match result {
            Ok(()) => {
                self.save_error = None;
                self.dirty = true;
                self.needs_projection = true;
                self.error = None;
            }
            Err(error) => self.error = Some(FormError::Runtime(*error)),
        }
    }
    pub fn command(&mut self, mut command: Command) {
        if !self.editable() {
            command.erase_input();
            return;
        }
        self.input_changed();
        self.needs_projection = true;
        self.pending.push(self.connection.command(command));
        self.dirty = true;
        self.error = None;
    }
    pub fn binary(&mut self, edit: BinaryEdit) {
        match taypeer_services::new_operation_id() {
            Ok(operation) => self.command(Command::EditBinary {
                request: BinaryRequest {
                    target: BinaryTarget::Draft,
                    edit,
                    review: None,
                },
                operation,
            }),
            Err(_) => self.error = Some(FormError::Backend),
        }
    }
    /// Drain only completed commands; this method never waits for IPC on the UI thread.
    pub fn poll(&mut self) -> bool {
        let mut changed = false;
        self.pending.retain(|ticket| match ticket.try_take() {
            None => true,
            Some(result) => {
                changed = true;
                match result {
                    Err(error) => self.error = Some(FormError::Runtime(error)),
                    Ok(()) => {
                        self.save_error = None;
                    }
                }
                false
            }
        });
        if let Some((revision, result)) = self
            .projection
            .as_ref()
            .and_then(|(revision, ticket)| ticket.try_take().map(|result| (*revision, result)))
        {
            self.projection = None;
            changed = true;
            match result {
                Ok(view) if revision == self.revision => self.install(view),
                Ok(_) => self.needs_projection = true,
                Err(error) => self.error = Some(FormError::Runtime(error)),
            }
        }
        if self.error.is_some() && self.pending.is_empty() {
            self.needs_projection = false;
        }
        if self.needs_projection
            && self.pending.is_empty()
            && self.forms_pending == 0
            && self.error.is_none()
            && self.projection.is_none()
        {
            self.needs_projection = false;
            self.projection = Some((self.revision, self.connection.command(Command::EditorView)));
        }
        if let Some((revision, attribute, result)) =
            self.reveal
                .as_ref()
                .and_then(|(revision, attribute, ticket)| {
                    ticket
                        .try_take()
                        .map(|result| (*revision, attribute.clone(), result))
                })
        {
            self.reveal = None;
            changed = true;
            match result {
                Ok(value) if revision == self.revision => {
                    if let Some(id) = attribute {
                        if let Some(field) = self
                            .content
                            .attributes
                            .iter_mut()
                            .find(|a| a.id.as_ref() == Some(&id))
                        {
                            field.value.zeroize();
                            field.value = value.to_string();
                        }
                    } else {
                        self.content.password.zeroize();
                        self.content.password = value.to_string();
                    }
                }
                Ok(_) => {}
                Err(error) => self.error = Some(FormError::Runtime(error)),
            }
        }
        changed |= self.poll_snapshot();
        changed
    }
    pub fn reveal(&mut self, attribute: Option<taypeer_core::AttributeId>) {
        if !self.busy() {
            self.reveal = Some((
                self.revision,
                attribute.clone(),
                self.connection.command(Command::RevealEditor(attribute)),
            ));
        }
    }
    pub fn clear_field(&mut self, field: EntryField) {
        if !self.editable() {
            return;
        }
        if field == EntryField::Password {
            self.content.password.zeroize();
        }
        field.set(&mut self.content, String::new());
        let mut patch = EntryPatch::default();
        match field {
            EntryField::Title => patch.title = FieldUpdate::Set(String::new()),
            EntryField::Username => patch.username = FieldUpdate::Clear,
            EntryField::Password => {
                self.content.has_password = false;
                patch.password = FieldUpdate::Clear;
            }
            EntryField::Url => patch.url = FieldUpdate::Clear,
            EntryField::Tags => patch.tags = FieldUpdate::Clear,
            EntryField::Notes => patch.notes = FieldUpdate::Clear,
            EntryField::Expires => {
                patch.expires_at = FieldUpdate::Clear;
                self.command(Command::DraftExpiry(None));
            }
        }
        self.command(Command::PatchDraft(patch));
    }
    fn input_changed(&mut self) {
        self.revision += 1;
        self.last_input = Instant::now();
    }
    pub fn set_field(&mut self, field: EntryField, value: String) {
        if !self.editable() || field.value(&self.content) == value {
            return;
        }
        if field == EntryField::Password {
            self.content.password.zeroize();
        }
        field.set(&mut self.content, value.clone());
        let mut patch = EntryPatch::default();
        match field {
            EntryField::Title => patch.title = FieldUpdate::Set(value),
            EntryField::Username => patch.username = FieldUpdate::Set(value),
            EntryField::Password => {
                self.content.has_password = true;
                patch.password = FieldUpdate::Set(value);
            }
            EntryField::Url => patch.url = FieldUpdate::Set(value),
            EntryField::Tags => {
                patch.tags = FieldUpdate::Set(value.lines().map(String::from).collect())
            }
            EntryField::Notes => patch.notes = FieldUpdate::Set(value),
            EntryField::Expires => {
                let parsed = if value.is_empty() {
                    Ok(None)
                } else {
                    chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M:%S%.f")
                        .or_else(|_| {
                            chrono::NaiveDateTime::parse_from_str(&value, "%Y-%m-%d %H:%M")
                        })
                        .map(|value| Some(value.and_utc().timestamp_millis()))
                };
                match parsed {
                    Ok(value) => {
                        self.command(Command::DraftExpiry(None));
                        patch.expires_at = value.map_or(FieldUpdate::Clear, FieldUpdate::Set);
                    }
                    Err(_) => {
                        self.command(Command::DraftExpiry(Some(value)));
                        return;
                    }
                }
            }
        }
        self.command(Command::PatchDraft(patch));
    }
    pub fn set_attribute_protected(&mut self, id: &taypeer_core::AttributeId, protected: bool) {
        if !self.editable() {
            return;
        }
        let Some(attribute) = self
            .content
            .attributes
            .iter_mut()
            .find(|attribute| attribute.id.as_ref() == Some(id))
        else {
            return;
        };
        if attribute.protected == protected {
            return;
        }
        attribute.protected = protected;
        let patch = taypeer_services::AttributePatch {
            id: attribute.id.clone(),
            name: attribute.key.clone(),
            value: FieldUpdate::Keep,
            protected,
        };
        self.command(Command::PatchAttribute {
            patch,
            remove: false,
        });
    }
    pub fn remove_attribute(&mut self, id: &taypeer_core::AttributeId) {
        if !self.editable() {
            return;
        }
        let Some(index) = self
            .content
            .attributes
            .iter()
            .position(|attribute| attribute.id.as_ref() == Some(id))
        else {
            return;
        };
        let attribute = self.content.attributes.remove(index);
        let patch = taypeer_services::AttributePatch {
            id: attribute.id.clone(),
            name: attribute.key.clone(),
            value: FieldUpdate::Keep,
            protected: attribute.protected,
        };
        self.command(Command::PatchAttribute {
            patch,
            remove: true,
        });
    }
    pub fn remove_attachment(&mut self, id: &taypeer_core::AttachmentId) {
        if !self.editable() {
            return;
        }
        let Some(index) = self
            .content
            .attachments
            .iter()
            .position(|attachment| &attachment.id == id)
        else {
            return;
        };
        let attachment = self.content.attachments.remove(index);
        self.binary(BinaryEdit::Attachment(
            taypeer_services::AttachmentEdit::Remove {
                attachment: attachment.id,
            },
        ));
    }
    pub fn set_color(&mut self, background: bool, value: Option<u32>) {
        if !self.editable() {
            return;
        }
        let target = if background {
            &mut self.content.background
        } else {
            &mut self.content.foreground
        };
        if *target == value {
            return;
        }
        *target = value;
        let update = value
            .map(|value| taypeer_core::Color(value.to_be_bytes()))
            .map_or(FieldUpdate::Clear, FieldUpdate::Set);
        self.binary(BinaryEdit::Appearance {
            foreground: if background {
                FieldUpdate::Keep
            } else {
                update.clone()
            },
            background: if background {
                update
            } else {
                FieldUpdate::Keep
            },
        });
    }
    pub fn set_icon(&mut self, name: String) {
        if !self.editable() || self.content.icon == name {
            return;
        }
        let Ok(key) = name.clone().try_into() else {
            return;
        };
        self.content.icon = name;
        self.binary(BinaryEdit::Icon(taypeer_services::IconInput::Lucide(key)));
    }
}
impl Drop for EditorStore {
    fn drop(&mut self) {
        self.content.password.zeroize();
        for a in &mut self.content.attributes {
            a.value.zeroize();
        }
    }
}
