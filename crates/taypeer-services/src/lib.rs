//! Session-checked scenarios for the first Taypeer application vertical.
//!
//! This service keeps public demonstration documents and drafts in memory. The
//! demonstration password is a public navigation gate, not authentication or
//! encryption. Lock revokes access through this API; it does not prove erasure of
//! Automerge allocations. No file format, storage, or network transport is used.

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use taypeer_core::{Attribute, AttributeValue, EntryFields, EntrySnapshot, Group, SavedRevision};
use taypeer_document::{Document, EntryDraft};

pub use taypeer_core::{AttributeId, DatabaseId, EntryId, GroupId, RevisionId};

/// Public input accepted by the synthetic unlock screen; never use a real password.
pub const DEMO_PASSWORD: &str = "SYNTHETIC-ONLY-Жук-42";

/// A logical marker for one generation of one unlocked demonstration database.
/// This public marker is not a cryptographic or unforgeable credential.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SessionToken {
    /// Database to which the command or response belongs.
    pub database: DatabaseId,
    /// Monotonically increasing local session generation.
    pub generation: u64,
}

/// A response that the UI must accept only while its session remains current.
#[derive(Clone, Debug)]
pub struct SessionValue<T> {
    /// Session that produced this response.
    pub session: SessionToken,
    /// Result of the command or read.
    pub value: T,
}

/// Non-secret catalog data available while a database is locked.
#[derive(Clone)]
pub struct DatabaseSummary {
    /// Stable database identifier.
    pub id: DatabaseId,
    /// Public demonstration database label.
    pub name: String,
    /// Whether a new unlock is needed for document access.
    pub locked: bool,
}

/// A group visible within an unlocked database.
#[derive(Clone)]
pub struct GroupSummary {
    /// Stable group identifier.
    pub id: GroupId,
    /// Group label.
    pub name: String,
    /// Parent group, or no parent for the visible top level.
    pub parent: Option<GroupId>,
}

/// A table/search row; password and protected attributes are never included.
#[derive(Clone)]
pub struct EntrySummary {
    /// Stable entry identifier.
    pub id: EntryId,
    /// Owning group.
    pub group_id: GroupId,
    /// Entry label; empty if unresolved conflicts prevent a unique view.
    pub title: String,
    /// Ordinary username, if present.
    pub username: Option<String>,
    /// Ordinary URL, if present.
    pub url: Option<String>,
    /// Whether the row requires conflict resolution before editing.
    pub has_conflicts: bool,
}

/// A search row with the database and group needed to navigate to its source.
#[derive(Clone)]
pub struct SearchResult {
    /// Public database label.
    pub database_name: String,
    /// Source group label.
    pub group_name: String,
    /// Matching entry without secret values.
    pub entry: EntrySummary,
}

/// Editable form retaining the distinction between absent and explicitly empty fields.
#[derive(Clone, Default, PartialEq, Eq)]
pub struct EditableEntry {
    /// Required entry label.
    pub title: String,
    /// Optional ordinary username.
    pub username: Option<String>,
    /// Optional secret; omitted from Debug output.
    pub password: Option<String>,
    /// Optional ordinary URL.
    pub url: Option<String>,
    /// Optional notes.
    pub notes: Option<String>,
    /// Tags; saved as a set by the domain model.
    pub tags: Vec<String>,
    /// Optional expiration timestamp in UTC milliseconds since the Unix epoch.
    pub expires_at: Option<i64>,
    /// Attributes with stable identifiers and atomic value/protection pairs.
    pub attributes: Vec<EditableAttribute>,
}

impl fmt::Debug for EditableEntry {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EditableEntry { fields: [REDACTED] }")
    }
}

/// One editable attribute; new attributes receive an identifier on draft update.
#[derive(Clone, PartialEq, Eq)]
pub struct EditableAttribute {
    /// Existing identifier, or None only for a new attribute.
    pub id: Option<AttributeId>,
    /// Attribute label.
    pub name: String,
    /// Attribute value; never included in Debug output.
    pub value: String,
    /// Whether list/search/read-only views must hide the value.
    pub protected: bool,
}

impl fmt::Debug for EditableAttribute {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("EditableAttribute { fields: [REDACTED] }")
    }
}

/// The single local form currently associated with a database.
#[derive(Clone)]
pub struct DraftView {
    /// Existing entry identifier; None denotes a not-yet-saved entry.
    pub entry_id: Option<EntryId>,
    /// Group in which the entry is being edited or created.
    pub group_id: GroupId,
    /// Form fields, including explicitly edited secrets.
    pub fields: EditableEntry,
    /// Whether the form differs from its starting state.
    pub dirty: bool,
    /// Unparsed expiration input retained for restoring an interrupted invalid form.
    /// None means the platform has supplied a valid optional timestamp.
    pub expiry_input: Option<String>,
}

/// Non-secret identity of an interrupted editor awaiting explicit restoration.
#[derive(Clone, Debug)]
pub struct PendingDraftSummary {
    /// Existing entry, or None for a not-yet-saved entry.
    pub entry_id: Option<EntryId>,
    /// Group containing the interrupted form.
    pub group_id: GroupId,
}

/// An attribute in a read-only view; protected values remain absent.
#[derive(Clone)]
pub struct AttributeView {
    /// Stable attribute identifier.
    pub id: AttributeId,
    /// Attribute label.
    pub name: String,
    /// Value only when the attribute is not protected.
    pub value: Option<String>,
    /// Whether explicit reveal is required.
    pub protected: bool,
}

/// Read-only entry details with masked password and protected attributes.
#[derive(Clone)]
pub struct EntryView {
    /// Stable entry identifier.
    pub id: EntryId,
    /// Owning group.
    pub group_id: GroupId,
    /// Entry label; empty for an ambiguous snapshot.
    pub title: String,
    /// Optional ordinary username.
    pub username: Option<String>,
    /// Optional ordinary URL.
    pub url: Option<String>,
    /// Optional ordinary notes.
    pub notes: Option<String>,
    /// Tags.
    pub tags: Vec<String>,
    /// Optional expiration timestamp in UTC milliseconds since the Unix epoch.
    pub expires_at: Option<i64>,
    /// Attributes with protected values removed.
    pub attributes: Vec<AttributeView>,
    /// Whether a password field exists, including an explicitly empty one.
    pub has_password: bool,
    /// Whether conflict resolution is required before editing or revealing.
    pub has_conflicts: bool,
    /// Creation timestamp in UTC milliseconds since the Unix epoch.
    pub created_at: i64,
    /// Modification timestamp in UTC milliseconds since the Unix epoch.
    pub modified_at: i64,
}

/// A history row with no secret values.
#[derive(Clone)]
pub struct RevisionSummary {
    /// Stable saved revision identifier.
    pub id: RevisionId,
    /// Saved entry title, or empty when ambiguous.
    pub title: String,
    /// Explicit-save timestamp in UTC milliseconds since the Unix epoch.
    pub saved_at: i64,
}

macro_rules! redacted_debug {
    ($($kind:ty),+ $(,)?) => {
        $(impl fmt::Debug for $kind {
            fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
                formatter.write_str(concat!(stringify!($kind), " { contents: [REDACTED] }"))
            }
        })+
    };
}

redacted_debug!(
    DatabaseSummary,
    GroupSummary,
    EntrySummary,
    SearchResult,
    DraftView,
    AttributeView,
    EntryView,
    RevisionSummary,
);

/// A value returned only by explicit reveal; formatting never prints its contents.
#[derive(Clone)]
pub struct SecretValue(String);

impl SecretValue {
    /// Borrow the revealed text for a current, explicitly requested UI presentation.
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for SecretValue {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("SecretValue([REDACTED])")
    }
}

/// Structured error categories containing no form values or credentials.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ServiceError {
    /// Requested database, group, entry, attribute, or revision does not exist.
    NotFound,
    /// The selected database is locked.
    Locked,
    /// A request marker belongs to an earlier session.
    ExpiredSession,
    /// The public demonstration password did not match.
    IncorrectDemoPassword,
    /// A required name is empty or another domain validation rule failed.
    InvalidInput,
    /// A different entry already owns this database's editor.
    EditorAlreadyOpen,
    /// No draft is available for the requested command.
    NoDraft,
    /// An interrupted editor must explicitly be restored or discarded first.
    DraftNeedsRestore,
    /// A conflicted document cannot supply a unique editable or revealed value.
    Conflict,
    /// A malformed or unrecognized draft context was rejected.
    InvalidContext,
    /// The session counter cannot allocate another generation.
    SessionExhausted,
    /// The in-memory document failed validation.
    InvalidDocument,
}

impl fmt::Display for ServiceError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(match self {
            Self::NotFound => "the requested object does not exist",
            Self::Locked => "the database is locked",
            Self::ExpiredSession => "the request belongs to an expired session",
            Self::IncorrectDemoPassword => "the public demonstration password did not match",
            Self::InvalidInput => "the input does not satisfy the domain rules",
            Self::EditorAlreadyOpen => "another entry already has an open editor",
            Self::NoDraft => "no entry draft is open",
            Self::DraftNeedsRestore => "the interrupted draft must be restored or discarded",
            Self::Conflict => "the entry has unresolved conflicts",
            Self::InvalidContext => "the draft context is invalid",
            Self::SessionExhausted => "no further session generation is available",
            Self::InvalidDocument => "the document is invalid",
        })
    }
}

impl std::error::Error for ServiceError {}

impl From<taypeer_document::Error> for ServiceError {
    fn from(error: taypeer_document::Error) -> Self {
        match error {
            taypeer_document::Error::Validation(_) => Self::InvalidInput,
            taypeer_document::Error::NotFound => Self::NotFound,
            taypeer_document::Error::Conflict => Self::Conflict,
            taypeer_document::Error::InvalidContext => Self::InvalidContext,
            taypeer_document::Error::DuplicateId => Self::InvalidInput,
            taypeer_document::Error::InvalidDocument => Self::InvalidDocument,
        }
    }
}

struct DraftState {
    document: EntryDraft,
    baseline: EditableEntry,
    is_new: bool,
    needs_restore: bool,
    attribute_order: Vec<AttributeId>,
    expiry_input: Option<String>,
}

impl DraftState {
    fn view(&self) -> DraftView {
        let mut fields = editable(self.document.fields());
        let dirty = fields != self.baseline;
        let order: BTreeMap<_, _> = self
            .attribute_order
            .iter()
            .enumerate()
            .map(|(index, id)| (id, index))
            .collect();
        fields.attributes.sort_by_key(|attribute| {
            attribute
                .id
                .as_ref()
                .and_then(|id| order.get(id))
                .copied()
                .unwrap_or(usize::MAX)
        });
        DraftView {
            entry_id: (!self.is_new).then(|| self.document.entry_id().clone()),
            group_id: self.document.group_id().clone(),
            dirty: dirty || self.expiry_input.is_some(),
            fields,
            expiry_input: self.expiry_input.clone(),
        }
    }
}

struct DatabaseState {
    document: Document,
    generation: u64,
    unlocked: bool,
    // Retained only in process memory when locked; this is not an encrypted draft.
    draft: Option<DraftState>,
}

/// Serialized in-memory application scenarios over the real domain/document model.
pub struct DemoService {
    databases: BTreeMap<DatabaseId, DatabaseState>,
    clock: fn() -> i64,
}

impl Default for DemoService {
    fn default() -> Self {
        Self::new()
    }
}

impl DemoService {
    /// Create an empty demonstration catalog.
    pub fn new() -> Self {
        Self::with_clock(now_millis)
    }

    /// Create a catalog with a caller-controlled UTC-milliseconds clock for verification.
    pub fn with_clock(clock: fn() -> i64) -> Self {
        Self {
            databases: BTreeMap::new(),
            clock,
        }
    }

    /// Build one locked database containing deliberately public example entries.
    pub fn with_sample_database() -> Result<Self, ServiceError> {
        let mut service = Self::new();
        let database = service.create_database("Demo / Демонстрация")?;
        let session = service.unlock(&database, DEMO_PASSWORD)?;
        let group = service
            .create_group(&session, "Examples / Примеры".into(), None)?
            .value;
        service.start_create_entry(&session, group.id)?;
        service.update_draft(
            &session,
            EditableEntry {
                title: "Public example / Публичный пример".into(),
                username: Some("demo@example.invalid".into()),
                password: Some("PUBLIC synthetic entry password".into()),
                url: Some("https://example.invalid".into()),
                notes: Some("Public demonstration data / Публичные демонстрационные данные".into()),
                tags: vec!["demo".into()],
                attributes: vec![EditableAttribute {
                    id: None,
                    name: "Public fixture / Публичный образец".into(),
                    value: "PUBLIC synthetic protected value".into(),
                    protected: true,
                }],
                ..EditableEntry::default()
            },
        )?;
        service.save_draft(&session)?;
        service.lock(&session)?;
        Ok(service)
    }

    /// List public database labels and their lock state.
    pub fn databases(&self) -> Vec<DatabaseSummary> {
        self.databases
            .iter()
            .map(|(id, state)| DatabaseSummary {
                id: id.clone(),
                name: state.document.name().into(),
                locked: !state.unlocked,
            })
            .collect()
    }

    /// Create a locked, empty database with no visible root or implicit group.
    pub fn create_database(&mut self, name: impl Into<String>) -> Result<DatabaseId, ServiceError> {
        let document = Document::new(name.into(), (self.clock)())?;
        let id = document.database_id().clone();
        self.databases.insert(
            id.clone(),
            DatabaseState {
                document,
                generation: 0,
                unlocked: false,
                draft: None,
            },
        );
        Ok(id)
    }

    /// Open a new demonstration session using the documented public input.
    pub fn unlock(
        &mut self,
        database: &DatabaseId,
        password: &str,
    ) -> Result<SessionToken, ServiceError> {
        let state = self
            .databases
            .get_mut(database)
            .ok_or(ServiceError::NotFound)?;
        if password != DEMO_PASSWORD {
            return Err(ServiceError::IncorrectDemoPassword);
        }
        let generation = state
            .generation
            .checked_add(1)
            .ok_or(ServiceError::SessionExhausted)?;
        state.generation = generation;
        state.unlocked = true;
        Ok(SessionToken {
            database: database.clone(),
            generation,
        })
    }

    /// Revoke document access and retain the draft in this process only.
    pub fn lock(&mut self, session: &SessionToken) -> Result<(), ServiceError> {
        let state = self.checked_mut(session)?;
        stash_draft(state);
        // Access closes even if incrementing the generation is no longer possible.
        state.unlocked = false;
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(ServiceError::SessionExhausted)?;
        Ok(())
    }

    /// Revoke every open session, for application lifecycle events.
    pub fn lock_all(&mut self) {
        for state in self.databases.values_mut() {
            if state.unlocked {
                stash_draft(state);
                state.unlocked = false;
                state.generation = state.generation.saturating_add(1);
            }
        }
    }

    /// Check whether a response's source database is still in the same unlocked session.
    pub fn is_current(&self, session: &SessionToken) -> bool {
        self.checked(session).is_ok()
    }

    /// Accept a response only for the selected session, which must also remain current.
    pub fn accepts_response(&self, active: &SessionToken, response: &SessionToken) -> bool {
        active == response && self.is_current(response)
    }

    /// List this session's groups without creating an implicit root.
    pub fn groups(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Vec<GroupSummary>>, ServiceError> {
        let groups = self.checked(session)?.document.groups()?;
        Ok(stamped(
            session,
            groups.into_iter().map(group_summary).collect(),
        ))
    }

    /// Create a group, optionally beneath an existing group in the same database.
    pub fn create_group(
        &mut self,
        session: &SessionToken,
        name: String,
        parent: Option<GroupId>,
    ) -> Result<SessionValue<GroupSummary>, ServiceError> {
        let now = (self.clock)();
        let group = self
            .checked_mut(session)?
            .document
            .create_group(name, parent, now)?;
        Ok(stamped(session, group_summary(group)))
    }

    /// Rename a group without changing its parent or the active entry draft.
    pub fn update_group(
        &mut self,
        session: &SessionToken,
        id: &GroupId,
        name: String,
    ) -> Result<SessionValue<GroupSummary>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        state.document.rename_group(id, name, now)?;
        let group = state
            .document
            .groups()?
            .into_iter()
            .find(|group| &group.id == id)
            .ok_or(ServiceError::NotFound)?;
        Ok(stamped(session, group_summary(group)))
    }

    /// List the selected group's entries, or search the whole database for a nonempty query.
    pub fn entries(
        &self,
        session: &SessionToken,
        group: Option<&GroupId>,
        query: &str,
    ) -> Result<SessionValue<Vec<EntrySummary>>, ServiceError> {
        let state = self.checked(session)?;
        if let Some(group_id) = group.filter(|_| query.is_empty())
            && !state
                .document
                .groups()?
                .iter()
                .any(|group| &group.id == group_id)
        {
            return Err(ServiceError::NotFound);
        }
        let query = query.to_lowercase();
        let mut entries: Vec<_> = state
            .document
            .entries()?
            .into_iter()
            .filter(|entry| !query.is_empty() || group.is_none_or(|group| &entry.group_id == group))
            .filter(|entry| matches_query(entry, &query))
            .map(entry_summary)
            .collect();
        entries.sort_by(|left, right| {
            left.title
                .to_lowercase()
                .cmp(&right.title.to_lowercase())
                .then_with(|| left.id.cmp(&right.id))
        });
        Ok(stamped(session, entries))
    }

    /// Search only currently unlocked databases, stamping every row with its source session.
    pub fn search_unlocked(
        &self,
        query: &str,
    ) -> Result<Vec<SessionValue<SearchResult>>, ServiceError> {
        let mut results = Vec::new();
        for (database, state) in &self.databases {
            if !state.unlocked {
                continue;
            }
            let session = SessionToken {
                database: database.clone(),
                generation: state.generation,
            };
            let groups: BTreeMap<_, _> = state
                .document
                .groups()?
                .into_iter()
                .map(|group| (group.id, group.name))
                .collect();
            for entry in self.entries(&session, None, query)?.value {
                let group_name = groups
                    .get(&entry.group_id)
                    .cloned()
                    .ok_or(ServiceError::InvalidDocument)?;
                results.push(stamped(
                    &session,
                    SearchResult {
                        database_name: state.document.name().into(),
                        group_name,
                        entry,
                    },
                ));
            }
        }
        results.sort_by(|left, right| {
            left.value
                .entry
                .title
                .to_lowercase()
                .cmp(&right.value.entry.title.to_lowercase())
                .then_with(|| left.session.database.cmp(&right.session.database))
                .then_with(|| left.value.entry.id.cmp(&right.value.entry.id))
        });
        Ok(results)
    }

    /// Read an entry without disclosing its password or protected attribute values.
    pub fn view_entry(
        &self,
        session: &SessionToken,
        id: &EntryId,
    ) -> Result<SessionValue<EntryView>, ServiceError> {
        Ok(stamped(
            session,
            entry_view(self.checked(session)?.document.entry(id)?),
        ))
    }

    /// Start a local unsaved entry form; another editor must first be saved or cancelled.
    pub fn start_create_entry(
        &mut self,
        session: &SessionToken,
        group: GroupId,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let document = state.document.begin_create_entry(group)?;
        let attribute_order = document.fields().attributes.keys().cloned().collect();
        let draft = DraftState {
            baseline: editable(document.fields()),
            document,
            is_new: true,
            needs_restore: false,
            attribute_order,
            expiry_input: None,
        };
        let view = draft.view();
        state.draft = Some(draft);
        Ok(stamped(session, view))
    }

    /// Open an explicit editor, or return the already open editor for the same entry.
    pub fn start_edit_entry(
        &mut self,
        session: &SessionToken,
        id: &EntryId,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let state = self.checked_mut(session)?;
        if let Some(draft) = &state.draft {
            if draft.needs_restore {
                return Err(ServiceError::DraftNeedsRestore);
            }
            if !draft.is_new && draft.document.entry_id() == id {
                return Ok(stamped(session, draft.view()));
            }
            return Err(ServiceError::EditorAlreadyOpen);
        }
        let document = state.document.begin_edit_entry(id)?;
        let attribute_order = document.fields().attributes.keys().cloned().collect();
        let draft = DraftState {
            baseline: editable(document.fields()),
            document,
            is_new: false,
            needs_restore: false,
            attribute_order,
            expiry_input: None,
        };
        let view = draft.view();
        state.draft = Some(draft);
        Ok(stamped(session, view))
    }

    /// Read the active local editor; an interrupted draft needs explicit restoration.
    pub fn draft(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Option<DraftView>>, ServiceError> {
        Ok(stamped(
            session,
            self.checked(session)?
                .draft
                .as_ref()
                .filter(|draft| !draft.needs_restore)
                .map(DraftState::view),
        ))
    }

    /// Inspect whether an interrupted form exists without exposing its values.
    pub fn pending_draft(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Option<PendingDraftSummary>>, ServiceError> {
        let pending = self
            .checked(session)?
            .draft
            .as_ref()
            .filter(|draft| draft.needs_restore)
            .map(|draft| PendingDraftSummary {
                entry_id: (!draft.is_new).then(|| draft.document.entry_id().clone()),
                group_id: draft.document.group_id().clone(),
            });
        Ok(stamped(session, pending))
    }

    /// Restore an interrupted editor after explicit user choice, using the new session stamp.
    pub fn restore_draft(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let draft = self
            .checked_mut(session)?
            .draft
            .as_mut()
            .ok_or(ServiceError::NoDraft)?;
        if !draft.needs_restore {
            return Err(ServiceError::InvalidContext);
        }
        draft.needs_restore = false;
        Ok(stamped(session, draft.view()))
    }

    /// Replace local form values without creating a document change or history row.
    pub fn update_draft(
        &mut self,
        session: &SessionToken,
        fields: EditableEntry,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let draft = self
            .checked_mut(session)?
            .draft
            .as_mut()
            .ok_or(ServiceError::NoDraft)?;
        if draft.needs_restore {
            return Err(ServiceError::DraftNeedsRestore);
        }
        // Work on a clone so invalid identities or duplicate attributes leave the editor intact.
        let mut candidate = draft.document.clone();
        let known: BTreeSet<_> = candidate.fields().attributes.keys().cloned().collect();
        let mut attributes = BTreeMap::new();
        let mut attribute_order = Vec::new();
        for attribute in fields.attributes {
            let id = match attribute.id {
                Some(id) if known.contains(&id) => id,
                Some(_) => return Err(ServiceError::InvalidContext),
                None => candidate.add_attribute(
                    attribute.name.clone(),
                    attribute.value.clone(),
                    attribute.protected,
                ),
            };
            attribute_order.push(id.clone());
            if attributes
                .insert(
                    id.clone(),
                    Attribute {
                        id,
                        name: attribute.name,
                        value: AttributeValue {
                            value: attribute.value,
                            protected: attribute.protected,
                        },
                    },
                )
                .is_some()
            {
                return Err(ServiceError::InvalidInput);
            }
        }
        *candidate.fields_mut() = EntryFields {
            title: fields.title,
            username: fields.username,
            password: fields.password,
            url: fields.url,
            notes: fields.notes,
            tags: fields.tags.into_iter().collect(),
            expires_at: fields.expires_at,
            attributes,
        };
        draft.document = candidate;
        draft.attribute_order = attribute_order;
        Ok(stamped(session, draft.view()))
    }

    /// Retain invalid platform date input without pretending it is a valid domain timestamp.
    /// Clear it after successful parsing; saving is rejected while Some remains.
    pub fn set_draft_expiry_input(
        &mut self,
        session: &SessionToken,
        input: Option<String>,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let draft = self
            .checked_mut(session)?
            .draft
            .as_mut()
            .ok_or(ServiceError::NoDraft)?;
        if draft.needs_restore {
            return Err(ServiceError::DraftNeedsRestore);
        }
        draft.expiry_input = input;
        Ok(stamped(session, draft.view()))
    }

    /// Confirm the form in memory; failed validation retains the draft and saved document.
    pub fn save_draft(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        let draft = state.draft.as_ref().ok_or(ServiceError::NoDraft)?;
        if draft.needs_restore {
            return Err(ServiceError::DraftNeedsRestore);
        }
        if draft.expiry_input.is_some() {
            return Err(ServiceError::InvalidInput);
        }
        let id = if !draft.is_new && !draft.view().dirty {
            draft.document.entry_id().clone()
        } else {
            state.document.save_entry(draft.document.clone(), now)?
        };
        state.draft = None;
        Ok(stamped(session, id))
    }

    /// Discard only the local editor, leaving saved data and history unchanged.
    pub fn cancel_draft(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<()>, ServiceError> {
        let state = self.checked_mut(session)?;
        if state.draft.is_none() {
            return Err(ServiceError::NoDraft);
        }
        state.draft = None;
        Ok(stamped(session, ()))
    }

    /// List explicit saved revisions with no secret values.
    pub fn history(
        &self,
        session: &SessionToken,
        id: &EntryId,
    ) -> Result<SessionValue<Vec<RevisionSummary>>, ServiceError> {
        let revisions = self.checked(session)?.document.history(id)?;
        Ok(stamped(
            session,
            revisions
                .into_iter()
                .map(|revision| RevisionSummary {
                    id: revision.id,
                    title: revision
                        .snapshot
                        .fields
                        .as_ref()
                        .map_or_else(String::new, |fields| fields.title.clone()),
                    saved_at: revision.saved_at,
                })
                .collect(),
        ))
    }

    /// Read a saved revision using the same masking policy as the current entry.
    pub fn revision(
        &self,
        session: &SessionToken,
        entry: &EntryId,
        revision: &RevisionId,
    ) -> Result<SessionValue<EntryView>, ServiceError> {
        Ok(stamped(
            session,
            entry_view(self.find_revision(session, entry, revision)?.snapshot),
        ))
    }

    /// Explicitly reveal the current password under a current session marker.
    pub fn reveal_password(
        &self,
        session: &SessionToken,
        id: &EntryId,
    ) -> Result<SessionValue<SecretValue>, ServiceError> {
        let entry = self.checked(session)?.document.entry(id)?;
        Ok(stamped(session, password(entry)?))
    }

    /// Explicitly reveal a current attribute value under a current session marker.
    pub fn reveal_attribute(
        &self,
        session: &SessionToken,
        entry: &EntryId,
        attribute: &AttributeId,
    ) -> Result<SessionValue<SecretValue>, ServiceError> {
        let entry = self.checked(session)?.document.entry(entry)?;
        Ok(stamped(session, attribute_value(entry, attribute)?))
    }

    /// Explicitly reveal a saved revision's password.
    pub fn reveal_revision_password(
        &self,
        session: &SessionToken,
        entry: &EntryId,
        revision: &RevisionId,
    ) -> Result<SessionValue<SecretValue>, ServiceError> {
        Ok(stamped(
            session,
            password(self.find_revision(session, entry, revision)?.snapshot)?,
        ))
    }

    /// Explicitly reveal a saved revision's attribute value.
    pub fn reveal_revision_attribute(
        &self,
        session: &SessionToken,
        entry: &EntryId,
        revision: &RevisionId,
        attribute: &AttributeId,
    ) -> Result<SessionValue<SecretValue>, ServiceError> {
        Ok(stamped(
            session,
            attribute_value(
                self.find_revision(session, entry, revision)?.snapshot,
                attribute,
            )?,
        ))
    }

    fn find_revision(
        &self,
        session: &SessionToken,
        entry: &EntryId,
        revision: &RevisionId,
    ) -> Result<SavedRevision, ServiceError> {
        self.checked(session)?
            .document
            .history(entry)?
            .into_iter()
            .find(|candidate| &candidate.id == revision)
            .ok_or(ServiceError::NotFound)
    }

    fn checked(&self, session: &SessionToken) -> Result<&DatabaseState, ServiceError> {
        let state = self
            .databases
            .get(&session.database)
            .ok_or(ServiceError::NotFound)?;
        check_session(state, session)?;
        Ok(state)
    }

    fn checked_mut(&mut self, session: &SessionToken) -> Result<&mut DatabaseState, ServiceError> {
        let state = self
            .databases
            .get_mut(&session.database)
            .ok_or(ServiceError::NotFound)?;
        check_session(state, session)?;
        Ok(state)
    }
}

fn check_session(state: &DatabaseState, session: &SessionToken) -> Result<(), ServiceError> {
    if !state.unlocked {
        return Err(ServiceError::Locked);
    }
    if state.generation != session.generation {
        return Err(ServiceError::ExpiredSession);
    }
    Ok(())
}

fn editor_open_error(state: &DatabaseState) -> ServiceError {
    if state
        .draft
        .as_ref()
        .is_some_and(|draft| draft.needs_restore)
    {
        ServiceError::DraftNeedsRestore
    } else {
        ServiceError::EditorAlreadyOpen
    }
}

fn stash_draft(state: &mut DatabaseState) {
    if let Some(draft) = &mut state.draft {
        if draft.view().dirty {
            draft.needs_restore = true;
        } else {
            state.draft = None;
        }
    }
}

fn stamped<T>(session: &SessionToken, value: T) -> SessionValue<T> {
    SessionValue {
        session: session.clone(),
        value,
    }
}

fn group_summary(group: Group) -> GroupSummary {
    GroupSummary {
        id: group.id,
        name: group.name,
        parent: group.parent,
    }
}

fn editable(fields: &EntryFields) -> EditableEntry {
    EditableEntry {
        title: fields.title.clone(),
        username: fields.username.clone(),
        password: fields.password.clone(),
        url: fields.url.clone(),
        notes: fields.notes.clone(),
        tags: fields.tags.iter().cloned().collect(),
        expires_at: fields.expires_at,
        attributes: fields
            .attributes
            .values()
            .map(|attribute| EditableAttribute {
                id: Some(attribute.id.clone()),
                name: attribute.name.clone(),
                value: attribute.value.value.clone(),
                protected: attribute.value.protected,
            })
            .collect(),
    }
}

fn matches_query(entry: &EntrySnapshot, query: &str) -> bool {
    if query.is_empty() {
        return true;
    }
    let Some(fields) = &entry.fields else {
        return false;
    };
    let contains = |value: &str| value.to_lowercase().contains(query);
    contains(&fields.title)
        || fields.username.as_deref().is_some_and(contains)
        || fields.url.as_deref().is_some_and(contains)
        || fields.notes.as_deref().is_some_and(contains)
        || fields.tags.iter().any(|tag| contains(tag))
        || fields
            .attributes
            .values()
            .any(|attribute| !attribute.value.protected && contains(&attribute.value.value))
}

fn entry_summary(entry: EntrySnapshot) -> EntrySummary {
    let has_conflicts = entry.fields.is_none();
    let fields = entry.fields.unwrap_or_default();
    EntrySummary {
        id: entry.id,
        group_id: entry.group_id,
        title: fields.title,
        username: fields.username,
        url: fields.url,
        has_conflicts,
    }
}

fn entry_view(entry: EntrySnapshot) -> EntryView {
    let has_conflicts = entry.fields.is_none();
    let fields = entry.fields.unwrap_or_default();
    EntryView {
        id: entry.id,
        group_id: entry.group_id,
        title: fields.title,
        username: fields.username,
        url: fields.url,
        notes: fields.notes,
        tags: fields.tags.into_iter().collect(),
        expires_at: fields.expires_at,
        attributes: fields
            .attributes
            .into_values()
            .map(|attribute| AttributeView {
                id: attribute.id,
                name: attribute.name,
                value: (!attribute.value.protected).then_some(attribute.value.value),
                protected: attribute.value.protected,
            })
            .collect(),
        has_password: fields.password.is_some(),
        has_conflicts,
        created_at: entry.created_at,
        modified_at: entry.modified_at,
    }
}

fn password(entry: EntrySnapshot) -> Result<SecretValue, ServiceError> {
    entry
        .fields
        .ok_or(ServiceError::Conflict)?
        .password
        .map(SecretValue)
        .ok_or(ServiceError::NotFound)
}

fn attribute_value(
    entry: EntrySnapshot,
    attribute: &AttributeId,
) -> Result<SecretValue, ServiceError> {
    entry
        .fields
        .ok_or(ServiceError::Conflict)?
        .attributes
        .remove(attribute)
        .map(|attribute| SecretValue(attribute.value.value))
        .ok_or(ServiceError::NotFound)
}

fn now_millis() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |duration| {
            i64::try_from(duration.as_millis()).unwrap_or(i64::MAX)
        })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn conflicted_snapshots_never_leak_an_implicit_winner_into_ui_views() {
        let mut service = DemoService::with_sample_database().unwrap();
        let database = service.databases()[0].id.clone();
        let session = service.unlock(&database, DEMO_PASSWORD).unwrap();
        let entry = service.entries(&session, None, "").unwrap().value[0]
            .id
            .clone();
        let mut other = service.databases[&database].document.fork();
        let mut foreign = other.begin_edit_entry(&entry).unwrap();
        foreign.fields_mut().password = Some("PUBLIC other branch".into());
        other.save_entry(foreign, 1_800_000_000_000).unwrap();
        let mut local = service
            .start_edit_entry(&session, &entry)
            .unwrap()
            .value
            .fields;
        local.password = Some("PUBLIC local branch".into());
        service.update_draft(&session, local).unwrap();
        service.save_draft(&session).unwrap();
        service
            .databases
            .get_mut(&database)
            .unwrap()
            .document
            .merge(&other)
            .unwrap();

        let view = service.view_entry(&session, &entry).unwrap().value;
        assert!(view.has_conflicts);
        assert!(view.title.is_empty());
        assert!(!view.has_password);
        assert!(view.attributes.is_empty());
        assert!(service.entries(&session, None, "").unwrap().value[0].has_conflicts);
        assert_eq!(
            service.start_edit_entry(&session, &entry).unwrap_err(),
            ServiceError::Conflict
        );
        assert_eq!(
            service.reveal_password(&session, &entry).unwrap_err(),
            ServiceError::Conflict
        );
        let history = service.history(&session, &entry).unwrap().value;
        assert_eq!(history.len(), 3);
        let passwords: BTreeSet<_> = history
            .iter()
            .map(|revision| {
                service
                    .reveal_revision_password(&session, &entry, &revision.id)
                    .unwrap()
                    .value
                    .expose()
                    .to_owned()
            })
            .collect();
        assert!(passwords.contains("PUBLIC other branch"));
        assert!(passwords.contains("PUBLIC local branch"));
    }

    #[test]
    fn exhausted_generation_still_closes_access_and_never_wraps() {
        let mut service = DemoService::new();
        let database = service
            .create_database("Public generation fixture")
            .unwrap();
        service.unlock(&database, DEMO_PASSWORD).unwrap();
        service.databases.get_mut(&database).unwrap().generation = u64::MAX;
        let last = SessionToken {
            database: database.clone(),
            generation: u64::MAX,
        };
        assert_eq!(
            service.lock(&last).unwrap_err(),
            ServiceError::SessionExhausted
        );
        assert!(!service.is_current(&last));
        assert_eq!(service.groups(&last).unwrap_err(), ServiceError::Locked);
        assert_eq!(
            service.unlock(&database, DEMO_PASSWORD).unwrap_err(),
            ServiceError::SessionExhausted
        );
    }
}
