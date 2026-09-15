//! Editable widget values and ordered delivery to the service-owned draft.
use super::*;
use crate::backend::{Connection, Ticket};
use taypeer_runtime::Command;
use taypeer_services::{
    BinaryEdit, BinaryRequest, BinaryTarget, EditorView, EntryPatch, FieldUpdate,
};
use zeroize::Zeroize;

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
    content: EntryContent,
    dirty: bool,
    preview: Option<taypeer_services::IconPreview>,
    error: Option<FormError>,
    pending: Vec<Ticket<()>>,
    projection: Option<(u64, Ticket<EditorView>)>,
    revision: u64,
    frozen: bool,
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
            content: EntryContent::default(),
            dirty: view.dirty,
            preview: None,
            error: None,
            pending: Vec::new(),
            projection: None,
            revision: 0,
            frozen: false,
            forms_pending: 0,
            needs_projection: false,
            reveal: None,
        };
        result.install(view);
        result
    }
    fn install(&mut self, view: EditorView) {
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
        !self.frozen && self.connection.control.is_open()
    }
    pub fn fail(&mut self, error: taypeer_runtime::RuntimeError) {
        self.frozen = false;
        self.error = Some(FormError::Runtime(error));
        self.needs_projection = false;
    }
    pub fn freeze(&mut self, value: bool) {
        self.frozen = value;
    }
    pub fn unacknowledged(&self) -> bool {
        !self.pending.is_empty() || self.forms_pending > 0
    }
    pub fn dirty(&self) -> bool {
        self.dirty
    }
    pub fn error(&self) -> Option<FormError> {
        self.error
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
        self.revision += 1;
        self.forms_pending += 1;
        self.connection.command(command)
    }
    pub fn finish_form(&mut self, result: &crate::backend::Result<()>) {
        self.forms_pending = self.forms_pending.saturating_sub(1);
        match result {
            Ok(()) => {
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
        self.revision += 1;
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
                if let Err(error) = result {
                    self.error = Some(FormError::Runtime(error));
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
                            field.value = value.to_string();
                        }
                    } else {
                        self.content.password = value.to_string();
                    }
                }
                Ok(_) => {}
                Err(error) => self.error = Some(FormError::Runtime(error)),
            }
        }
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
    pub fn edit(&mut self, edit: impl FnOnce(&mut EntryContent)) {
        if !self.editable() {
            return;
        }
        let old = self.content.clone();
        edit(&mut self.content);
        if self.content == old {
            return;
        }
        let mut patch = EntryPatch::default();
        macro_rules! changed {
            ($field:ident) => {
                if self.content.$field != old.$field {
                    patch.$field = FieldUpdate::Set(self.content.$field.clone());
                }
            };
        }
        changed!(title);
        changed!(username);
        changed!(password);
        changed!(url);
        changed!(notes);
        if self.content.tags != old.tags {
            patch.tags = FieldUpdate::Set(self.content.tags.lines().map(String::from).collect());
        }
        if self.content.expires != old.expires {
            let parsed = if self.content.expires.is_empty() {
                Ok(None)
            } else {
                chrono::NaiveDateTime::parse_from_str(&self.content.expires, "%Y-%m-%d %H:%M:%S%.f")
                    .or_else(|_| {
                        chrono::NaiveDateTime::parse_from_str(
                            &self.content.expires,
                            "%Y-%m-%d %H:%M",
                        )
                    })
                    .map(|v| Some(v.and_utc().timestamp_millis()))
            };
            match parsed {
                Ok(value) => {
                    patch.expires_at = value.map_or(FieldUpdate::Clear, FieldUpdate::Set);
                    self.command(Command::DraftExpiry(None));
                }
                Err(_) => self.command(Command::DraftExpiry(Some(self.content.expires.clone()))),
            }
        }
        self.command(Command::PatchDraft(patch));
        // Existing UI controls use the same edit entry point for presentation fields.
        if self.content.icon != old.icon
            && let Ok(key) = self.content.icon.clone().try_into()
        {
            self.binary(BinaryEdit::Icon(taypeer_services::IconInput::Lucide(key)));
        }
        if self.content.foreground != old.foreground || self.content.background != old.background {
            let color = |value: Option<u32>| {
                value
                    .map(|v| taypeer_core::Color([(v >> 16) as u8, (v >> 8) as u8, v as u8, 255]))
                    .map_or(FieldUpdate::Clear, FieldUpdate::Set)
            };
            self.binary(BinaryEdit::Appearance {
                foreground: color(self.content.foreground),
                background: color(self.content.background),
            });
        }
        for a in &old.attributes {
            if !self.content.attributes.iter().any(|b| b.id == a.id) {
                self.command(Command::PatchAttribute {
                    patch: taypeer_services::AttributePatch {
                        id: a.id.clone(),
                        name: a.key.clone(),
                        value: FieldUpdate::Keep,
                        protected: a.protected,
                    },
                    remove: true,
                });
            }
        }
        let updates: Vec<_> = self
            .content
            .attributes
            .iter()
            .filter(|a| !old.attributes.contains(a))
            .cloned()
            .collect();
        for a in updates {
            let same = old
                .attributes
                .iter()
                .find(|b| b.id == a.id && a.id.is_some());
            let value = if same.is_some_and(|b| b.value == a.value) {
                FieldUpdate::Keep
            } else {
                FieldUpdate::Set(a.value)
            };
            self.command(Command::PatchAttribute {
                patch: taypeer_services::AttributePatch {
                    id: a.id,
                    name: a.key,
                    value,
                    protected: a.protected,
                },
                remove: false,
            });
        }
        for a in &old.attachments {
            if !self.content.attachments.iter().any(|b| b.id == a.id) {
                self.binary(BinaryEdit::Attachment(
                    taypeer_services::AttachmentEdit::Remove {
                        attachment: a.id.clone(),
                    },
                ));
            }
        }
        let renamed: Vec<_> = self
            .content
            .attachments
            .iter()
            .filter(|a| {
                old.attachments
                    .iter()
                    .any(|b| a.id == b.id && a.name != b.name)
            })
            .cloned()
            .collect();
        for a in renamed {
            self.binary(BinaryEdit::Attachment(
                taypeer_services::AttachmentEdit::Rename {
                    attachment: a.id,
                    name: a.name,
                },
            ));
        }
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
