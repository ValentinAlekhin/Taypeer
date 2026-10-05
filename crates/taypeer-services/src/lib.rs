//! Session-checked scenarios for encrypted files and explicit volatile demonstrations.
//! File-backed commands publish candidates only after storage succeeds.
//! Lock releases file-backed plaintext and keys; physical erasure of Automerge is not proven.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use std::path::Path;
use taypeer_core::SavedRevision;
use taypeer_document::Document;
/// File operation failures exposed through the service boundary.
pub use taypeer_storage::Error as StorageError;
use taypeer_storage::{BlobStore, FileStore, ReadKey};
use zeroize::Zeroizing;

mod binary;
pub mod generator;
pub mod icons;
pub use binary::*;
mod lifecycle;
mod operations;
pub use lifecycle::{InspectionTarget, InspectionView, TreeView};
mod managed;
mod patch;
mod presentation;
pub use presentation::*;
mod persistence;
pub use managed::{
    ApplyReport, CollectionReport, EpochCredentialStage, PendingPacket, PendingReason,
    ReceivedSource,
};
pub use operations::{ConflictFieldView, ConflictVariantView, ConflictView, new_operation_id};
pub use patch::{EntryPatch, FieldUpdate};
pub use taypeer_document::{ConflictContext, Resolution};
pub use taypeer_document::{
    GroupMove, GroupNode, LifecycleAction, ObjectAddress, ObjectId, ObjectState, ObjectStatus,
    PendingSource, PreparedLifecycle, RecoveryMode, RecoveryRequest, SiblingPosition,
};

mod commands;
mod draft;
mod editing;
use draft::{DraftKind, DraftState};
use editing::DraftCollection;
pub use editing::{
    AUTOSAVE_DELAY_MILLIS, DraftIdentity, DraftRevision, DraftSaveOutcome, DraftSummary,
    DraftTarget, MetadataDraftView,
};

pub use taypeer_core::{AttributeId, DatabaseId, DraftId, EntryId, GroupId, RevisionId};

/// Public input accepted by the synthetic unlock screen; never use a real password.
pub const DEMO_PASSWORD: &str = "SYNTHETIC-ONLY-Жук-42";

/// A logical marker for one generation of one unlocked demonstration database.
/// This public marker is not a cryptographic or unforgeable credential.
#[derive(Clone, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct SessionToken {
    /// Database to which the command or response belongs.
    pub database: DatabaseId,
    /// Monotonically increasing local session generation.
    pub generation: u64,
}

/// A response that the UI must accept only while its session remains current.
#[derive(Clone, Debug, serde::Serialize, serde::Deserialize)]
pub struct SessionValue<T> {
    /// Session that produced this response.
    pub session: SessionToken,
    /// Result of the command or read.
    pub value: T,
}

mod views;
pub use views::{
    AttributeView, DatabaseSummary, DraftView, EditableAttribute, EditableEntry, EntrySummary,
    EntryView, GroupSummary, PendingDraftSummary, RevisionSummary, SearchResult, SecretValue,
};
use views::{attribute_value, entry_summary, entry_view, group_summary, matches_query, password};

/// Structured error categories containing no form values or credentials.
#[derive(
    Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize, thiserror::Error,
)]
pub enum ServiceError {
    /// Requested database, group, entry, attribute, or revision does not exist.
    #[error("the requested object does not exist")]
    NotFound,
    /// The operation ID is already bound to another exact request.
    #[error("the operation ID belongs to another request")]
    OperationConflict,
    /// The selected database is locked.
    #[error("the database is locked")]
    Locked,
    /// A request marker belongs to an earlier session.
    #[error("the request belongs to an expired session")]
    ExpiredSession,
    /// The public demonstration password did not match.
    #[error("the public demonstration password did not match")]
    IncorrectDemoPassword,
    /// A required name is empty or another domain validation rule failed.
    #[error("the input does not satisfy the domain rules")]
    InvalidInput,
    /// A different entry already owns this database's editor.
    #[error("another entry already has an open editor")]
    EditorAlreadyOpen,
    /// No draft is available for the requested command.
    #[error("no entry draft is open")]
    NoDraft,
    /// An interrupted editor must explicitly be restored or discarded first.
    #[error("the interrupted draft must be restored or discarded")]
    DraftNeedsRestore,
    /// A conflicted document cannot supply a unique editable or revealed value.
    #[error("the entry has unresolved conflicts")]
    Conflict,
    /// A malformed or unrecognized draft context was rejected.
    #[error("the draft context is invalid")]
    InvalidContext,
    /// The session counter cannot allocate another generation.
    #[error("no further session generation is available")]
    SessionExhausted,
    /// The in-memory document failed validation.
    #[error("the document is invalid")]
    InvalidDocument,
    /// New attachment contents exceed the current database quota.
    #[error("the attachment limit is exceeded")]
    AttachmentLimit,
    /// A copied or revoked profile has read/export access but no current author permission.
    #[error("the device has read-only access")]
    ReadOnly,
    /// This build cannot safely read the authenticated schema requirements.
    #[error("update Taypeer to read this database")]
    ReadCompatibility,
    /// This build may read the document but cannot safely change its schema semantics.
    #[error("update Taypeer to change this database")]
    WriteCompatibility,
    /// A signature or authenticated device binding is not authorized.
    #[error("the device is not authorized")]
    Unauthorized,
    /// Native author credential storage is unavailable or denied access.
    #[error("native credential storage is unavailable")]
    Credentials,
    /// New authority is known but its required encrypted baseline/content is still in transit.
    #[error("required encrypted data has not arrived")]
    AwaitingData,
    /// The signed authority chain rejected an operation.
    #[error("signed authority verification failed")]
    Trust(taypeer_trust::Error),
    /// An explicit image download or validation failed.
    #[error("the image could not be loaded")]
    Icon(icons::IconError),
    /// An encrypted file operation failed without exposing paths or secret content.
    #[error("the encrypted file operation failed")]
    Storage(taypeer_storage::Error),
}

impl From<taypeer_document::Error> for ServiceError {
    fn from(error: taypeer_document::Error) -> Self {
        match error {
            taypeer_document::Error::Validation(_) => Self::InvalidInput,
            taypeer_document::Error::NotFound => Self::NotFound,
            taypeer_document::Error::Conflict => Self::Conflict,
            taypeer_document::Error::InvalidContext => Self::InvalidContext,
            taypeer_document::Error::DuplicateId => Self::InvalidInput,
            taypeer_document::Error::InvalidDocument => Self::InvalidDocument,
            taypeer_document::Error::UnsupportedSchema => Self::ReadCompatibility,
            taypeer_document::Error::Random => Self::Storage(taypeer_storage::Error::Random),
        }
    }
}

struct DatabaseState {
    document: Option<Document>,
    blobs: Option<BlobStore>,
    label: String,
    file: Option<FileStore>,
    managed: Option<managed::ManagedState>,
    key: Option<ReadKey>,
    generation: u64,
    unlocked: bool,
    // Retained only in process memory when locked; this is not an encrypted draft.
    drafts: DraftCollection,
    // The on-disk draft was deliberately not decoded by an incompatible writer.
    draft_deferred: bool,
    draft_maintenance_pending: bool,
    write_uncertain: bool,
}

/// Serialized application scenarios over domain documents and optional encrypted file storage.
pub struct DatabaseService {
    databases: BTreeMap<DatabaseId, DatabaseState>,
    clock: fn() -> i64,
    capabilities: taypeer_core::ClientCapabilities,
}

/// Compatibility name for callers that explicitly create volatile demonstration databases.
pub type DemoService = DatabaseService;

impl Default for DatabaseService {
    fn default() -> Self {
        Self::new()
    }
}

impl DatabaseService {
    /// Create an empty demonstration catalog.
    pub fn new() -> Self {
        Self::with_clock(now_millis)
    }

    /// Create a catalog with a caller-controlled UTC-milliseconds clock for verification.
    pub fn with_clock(clock: fn() -> i64) -> Self {
        Self {
            databases: BTreeMap::new(),
            clock,
            capabilities: taypeer_core::ClientCapabilities::default(),
        }
    }

    /// Create a client with a restricted subset of this build's format capabilities.
    /// Production clients use `new`; this cannot enable unimplemented semantics.
    pub fn with_capabilities(capabilities: taypeer_core::ClientCapabilities) -> Self {
        Self {
            capabilities,
            ..Self::new()
        }
    }

    /// Build one locked database containing deliberately public example entries.
    pub fn with_sample_database() -> Result<Self, ServiceError> {
        let mut service = Self::new();
        let database = service.create_database("Demo / Демонстрация")?;
        let session = service.unlock(&database, DEMO_PASSWORD)?;
        let group = service
            .create_group(
                &session,
                "Examples / Примеры".into(),
                None,
                &crate::new_operation_id()?,
            )?
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
        service.save_draft(&session, &crate::new_operation_id()?)?;
        service.lock(&session)?;
        Ok(service)
    }

    /// List public database labels and their lock state.
    pub fn databases(&self) -> Vec<DatabaseSummary> {
        self.databases
            .iter()
            .map(|(id, state)| DatabaseSummary {
                id: id.clone(),
                name: state.label.clone(),
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
                label: document.name().into(),
                document: Some(document),
                blobs: Some(BlobStore::new()?),
                file: None,
                managed: None,
                key: None,
                generation: 0,
                unlocked: false,
                drafts: DraftCollection::new(id.clone()),
                draft_deferred: false,
                draft_maintenance_pending: false,
                write_uncertain: false,
            },
        );
        Ok(id)
    }

    /// Open a fresh session with the file master password or the public demo input.
    pub fn unlock(
        &mut self,
        database: &DatabaseId,
        password: &str,
    ) -> Result<SessionToken, ServiceError> {
        let state = self
            .databases
            .get_mut(database)
            .ok_or(ServiceError::NotFound)?;
        if state.managed.is_some() {
            return Err(ServiceError::InvalidContext);
        }
        if state.file.is_some() {
            return state.unlock_file(database, password.as_bytes(), &self.capabilities);
        }
        if password != DEMO_PASSWORD {
            return Err(ServiceError::IncorrectDemoPassword);
        }
        managed::compatibility::require_read(
            &self
                .capabilities
                .assess(&state.document().schema_descriptor()?),
        )?;
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

    /// Revoke document access; file-backed drafts are encrypted locally before plaintext is released.
    pub fn lock(&mut self, session: &SessionToken) -> Result<(), ServiceError> {
        let state = self
            .databases
            .get_mut(&session.database)
            .ok_or(ServiceError::NotFound)?;
        if !state.unlocked {
            return Err(ServiceError::Locked);
        }
        if state.generation != session.generation {
            return Err(ServiceError::ExpiredSession);
        }
        let draft_result = state.stash_and_close();
        // Access closes even if incrementing the generation is no longer possible.
        state.unlocked = false;
        state.generation = state
            .generation
            .checked_add(1)
            .ok_or(ServiceError::SessionExhausted)?;
        draft_result
    }

    /// Revoke every open session, for application lifecycle events.
    pub fn lock_all(&mut self) {
        // Compatibility lifecycle API. Call lock_all_checked when the UI can display a draft error.
        let _ = self.lock_all_checked();
    }

    /// Revoke every session even if a local draft cannot be saved, returning the first failure.
    pub fn lock_all_checked(&mut self) -> Result<(), ServiceError> {
        let mut result = Ok(());
        for state in self.databases.values_mut() {
            if state.unlocked {
                let closed = state.stash_and_close();
                state.unlocked = false;
                state.generation = state.generation.saturating_add(1);
                if result.is_ok() {
                    result = closed;
                }
            }
        }
        result
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
        let groups = self.checked(session)?.document().groups()?;
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
        operation: &taypeer_core::OperationId,
    ) -> Result<SessionValue<GroupSummary>, ServiceError> {
        let now = (self.clock)();
        let fingerprint = commands::fingerprint(&(name.as_str(), &parent))?;
        let group = self.checked_mut(session)?.command(
            operation,
            "create_group",
            fingerprint,
            |doc, receipt| Ok(doc.create_group_command(name, parent, now, Some(receipt))?),
        )?;
        Ok(stamped(session, group_summary(group)))
    }

    /// Rename a group; retry returns the original summary without reverting later edits.
    pub fn update_group(
        &mut self,
        session: &SessionToken,
        id: &GroupId,
        name: String,
        operation: &taypeer_core::OperationId,
    ) -> Result<SessionValue<GroupSummary>, ServiceError> {
        let now = (self.clock)();
        let fingerprint = commands::fingerprint(&(id, &name))?;
        let group = self.checked_mut(session)?.command(
            operation,
            "rename_group",
            fingerprint,
            |doc, receipt| Ok(doc.rename_group_command(id, name, now, Some(receipt))?),
        )?;
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
                .document()
                .groups()?
                .iter()
                .any(|group| &group.id == group_id)
        {
            return Err(ServiceError::NotFound);
        }
        let query = query.to_lowercase();
        let mut entries: Vec<_> = state
            .document()
            .entries()?
            .into_iter()
            .filter(|entry| {
                !query.is_empty()
                    || group.is_none_or(|group| entry.group_id.as_ref() == Some(group))
            })
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
                .document()
                .groups()?
                .into_iter()
                .map(|group| (group.id, group.name))
                .collect();
            for entry in self.entries(&session, None, query)?.value {
                let group_name = entry
                    .group_id
                    .as_ref()
                    .map(|id| groups.get(id).cloned().ok_or(ServiceError::InvalidDocument))
                    .transpose()?;
                results.push(stamped(
                    &session,
                    SearchResult {
                        database_name: state.label.clone(),
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
            entry_view(self.checked(session)?.document().entry(id)?),
        ))
    }

    /// Start a local unsaved entry form, parking any changed previous editor.
    pub fn start_create_entry(
        &mut self,
        session: &SessionToken,
        group: GroupId,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        self.start_create_entry_in(session, Some(group))
    }

    /// Begin a local form without requiring an existing group.
    pub fn start_create_entry_ungrouped(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        self.start_create_entry_in(session, None)
    }

    /// Begin a new local form at an optional destination, parking the previous editor.
    pub fn start_create_entry_in(
        &mut self,
        session: &SessionToken,
        group: Option<GroupId>,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let state = self.checked_mut(session)?;
        let document = state.document().begin_create_entry_in(group)?;
        let draft = DraftState::new(document, DraftKind::New)?;
        let view = draft.view();
        state.drafts.activate_entry(draft);
        Ok(stamped(session, view))
    }

    /// Open an editor and automatically continue the retained form for the same entry.
    pub fn start_edit_entry(
        &mut self,
        session: &SessionToken,
        id: &EntryId,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let state = self.checked_mut(session)?;
        if let Some(draft) = &mut state.drafts.entry
            && draft.entry_id() == Some(id)
        {
            draft.resume();
            return Ok(stamped(session, draft.view()));
        }
        let mut draft = match state.drafts.take_entry_for(id) {
            Some(draft) => draft,
            None => DraftState::new(state.document().begin_edit_entry(id)?, DraftKind::Existing)?,
        };
        draft.resume();
        let view = draft.view();
        state.drafts.activate_entry(draft);
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
                .drafts
                .entry
                .as_ref()
                .filter(|draft| !draft.needs_restore())
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
            .drafts
            .entry
            .as_ref()
            .filter(|draft| draft.needs_restore())
            .map(DraftState::pending_summary);
        Ok(stamped(session, pending))
    }

    /// Restore an interrupted editor after explicit user choice, using the new session stamp.
    pub fn restore_draft(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let draft = self
            .checked_mut(session)?
            .drafts
            .entry
            .as_mut()
            .ok_or(ServiceError::NoDraft)?;
        Ok(stamped(session, draft.restore()?))
    }

    /// Replace local form values without creating a document change or history row.
    pub fn update_draft(
        &mut self,
        session: &SessionToken,
        fields: EditableEntry,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let state = self.checked_mut(session)?;
        let mut draft = state.draft_for_edit()?;
        let view = draft.update(fields)?;
        state.drafts.entry = Some(draft);
        Ok(stamped(session, view))
    }

    /// Retain invalid platform date input without pretending it is a valid domain timestamp.
    /// Clear it after successful parsing; saving is rejected while Some remains.
    pub fn set_draft_expiry_input(
        &mut self,
        session: &SessionToken,
        input: Option<String>,
    ) -> Result<SessionValue<DraftView>, ServiceError> {
        let state = self.checked_mut(session)?;
        let mut draft = state.draft_for_edit()?;
        let view = draft.set_expiry_input(input)?;
        state.drafts.entry = Some(draft);
        Ok(stamped(session, view))
    }

    /// Confirm the form after storage succeeds; errors retain the draft and prior document.
    pub fn save_draft(
        &mut self,
        session: &SessionToken,
        operation: &taypeer_core::OperationId,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if let Some(id) = state.command_result::<EntryId>(operation, "save_draft", None)? {
            if let Some(draft) = &state.drafts.entry
                && draft
                    .attempt
                    .as_ref()
                    .is_some_and(|(id, _)| id == operation)
            {
                if !draft.confirmed(state.document())? {
                    return Err(ServiceError::OperationConflict);
                }
                state.clear_saved_draft()?;
            }
            return Ok(stamped(session, id));
        }
        let draft = state.drafts.entry.as_ref().ok_or(ServiceError::NoDraft)?;
        if draft.has_binary_operation(operation) {
            return Err(ServiceError::OperationConflict);
        }
        let fingerprint = draft.fingerprint()?;
        if draft
            .attempt
            .as_ref()
            .is_some_and(|(id, old)| id == operation && old != &fingerprint)
        {
            return Err(ServiceError::OperationConflict);
        }
        if draft.adds_binary_content() {
            let mut refs = state.document().blob_references()?.attachments;
            refs.extend(
                draft
                    .document()?
                    .fields()
                    .attachments
                    .values()
                    .map(|a| a.blob.clone()),
            );
            if state.blobs()?.unique_bytes(&refs) > state.policy().total_attachment_bytes() {
                return Err(ServiceError::AttachmentLimit);
            }
        }
        let mut candidate = state.document().clone();
        let receipt = taypeer_document::CommandReceipt {
            operation,
            kind: "save_draft",
            fingerprint: &fingerprint,
        };
        let id = draft.save_command(&mut candidate, now, &receipt)?;
        let mut bound = draft.clone();
        bound.attempt = Some((operation.clone(), fingerprint));
        state.persist_binary_draft(&bound, state.blobs()?)?;
        state.drafts.entry = Some(bound);
        state.commit(candidate)?;
        state.clear_saved_draft()?;
        Ok(stamped(session, id))
    }

    /// Discard only the local editor, leaving saved data and history unchanged.
    pub fn cancel_draft(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<()>, ServiceError> {
        let id = self
            .checked(session)?
            .drafts
            .entry
            .as_ref()
            .ok_or(ServiceError::NoDraft)?
            .identity()
            .draft;
        self.delete_draft(session, &id)
    }

    /// List explicit saved revisions with no secret values.
    pub fn history(
        &self,
        session: &SessionToken,
        id: &EntryId,
    ) -> Result<SessionValue<Vec<RevisionSummary>>, ServiceError> {
        let revisions = self.checked(session)?.document().history(id)?;
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
        let entry = self.checked(session)?.document().entry(id)?;
        Ok(stamped(session, password(entry)?))
    }

    /// Explicitly reveal a current attribute value under a current session marker.
    pub fn reveal_attribute(
        &self,
        session: &SessionToken,
        entry: &EntryId,
        attribute: &AttributeId,
    ) -> Result<SessionValue<SecretValue>, ServiceError> {
        let entry = self.checked(session)?.document().entry(entry)?;
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
            .document()
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
        if state.managed.is_none() {
            managed::compatibility::require_read(
                &self
                    .capabilities
                    .assess(&state.document().schema_descriptor()?),
            )?;
        }
        Ok(state)
    }

    fn checked_mut(&mut self, session: &SessionToken) -> Result<&mut DatabaseState, ServiceError> {
        let state = self
            .databases
            .get_mut(&session.database)
            .ok_or(ServiceError::NotFound)?;
        check_session(state, session)?;
        if state.write_uncertain {
            return Err(StorageError::CommitUncertain.into());
        }
        if let Some(managed) = &state.managed {
            managed.check_edit_permission()?;
        } else {
            managed::compatibility::require_write(
                &self
                    .capabilities
                    .assess(&state.document().schema_descriptor()?),
            )?;
        }
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
    if let Some(managed) = &state.managed {
        managed.check_session()?;
    }
    Ok(())
}

fn stash_draft(state: &mut DatabaseState) {
    state.drafts.stash();
}

fn stamped<T>(session: &SessionToken, value: T) -> SessionValue<T> {
    SessionValue {
        session: session.clone(),
        value,
    }
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
    use std::collections::BTreeSet;

    #[test]
    fn concurrent_snapshots_select_ordinary_values_and_preserve_original_history() {
        let mut service = DemoService::with_sample_database().unwrap();
        let database = service.databases()[0].id.clone();
        let session = service.unlock(&database, DEMO_PASSWORD).unwrap();
        let entry = service.entries(&session, None, "").unwrap().value[0]
            .id
            .clone();
        let mut other = service.databases[&database].document().fork();
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
        service
            .save_draft(&session, &crate::new_operation_id().unwrap())
            .unwrap();
        service
            .databases
            .get_mut(&database)
            .unwrap()
            .document
            .as_mut()
            .unwrap()
            .merge(&other)
            .unwrap();

        let view = service.view_entry(&session, &entry).unwrap().value;
        assert!(view.has_conflicts);
        assert!(!view.title.is_empty());
        assert!(view.has_password);
        assert!(service.entries(&session, None, "").unwrap().value[0].has_conflicts);
        service.start_edit_entry(&session, &entry).unwrap();
        assert!(
            service
                .editor_view(&session)
                .unwrap()
                .fields
                .password
                .is_none()
        );
        let selected = service.reveal_password(&session, &entry).unwrap();
        assert!(["PUBLIC other branch", "PUBLIC local branch"].contains(&selected.value.expose()));
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
