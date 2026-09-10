//! Session-checked scenarios for encrypted files and explicit volatile demonstrations.
//! File-backed commands publish candidates only after storage succeeds.
//! Lock releases file-backed plaintext and keys; physical erasure of Automerge is not proven.

use std::collections::BTreeMap;
use std::fmt;
use std::time::{SystemTime, UNIX_EPOCH};

use std::path::Path;
use taypeer_core::SavedRevision;
use taypeer_document::Document;
/// File operation failures exposed through the service boundary.
pub use taypeer_storage::Error as StorageError;
use taypeer_storage::{FileStore, ReadKey};
use zeroize::Zeroizing;

pub mod generator;
mod operations;
mod patch;
mod persistence;
pub use operations::{ConflictFieldView, ConflictVariantView, ConflictView, new_operation_id};
pub use patch::{EntryPatch, FieldUpdate};
pub use taypeer_document::{ConflictContext, Resolution};

mod draft;
use draft::{DraftKind, DraftState};

pub use taypeer_core::{AttributeId, DatabaseId, EntryId, GroupId, RevisionId};

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
#[derive(Clone, Copy, Debug, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
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
    /// An encrypted file operation failed without exposing paths or secret content.
    Storage(taypeer_storage::Error),
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
            Self::Storage(_) => "the encrypted file operation failed",
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

struct DatabaseState {
    document: Option<Document>,
    label: String,
    file: Option<FileStore>,
    key: Option<ReadKey>,
    generation: u64,
    unlocked: bool,
    // Retained only in process memory when locked; this is not an encrypted draft.
    draft: Option<DraftState>,
}

/// Serialized application scenarios over domain documents and optional encrypted file storage.
pub struct DatabaseService {
    databases: BTreeMap<DatabaseId, DatabaseState>,
    clock: fn() -> i64,
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
                file: None,
                key: None,
                generation: 0,
                unlocked: false,
                draft: None,
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
        if state.file.is_some() {
            return state.unlock_file(database, password.as_bytes());
        }
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

    /// Revoke document access; file-backed drafts are encrypted locally before plaintext is released.
    pub fn lock(&mut self, session: &SessionToken) -> Result<(), ServiceError> {
        let state = self.checked_mut(session)?;
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
    ) -> Result<SessionValue<GroupSummary>, ServiceError> {
        let now = (self.clock)();
        let group = self
            .checked_mut(session)?
            .change(|document| Ok(document.create_group(name, parent, now)?))?;
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
        state.change(|document| Ok(document.rename_group(id, name, now)?))?;
        let group = state
            .document()
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
                .document()
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
        let document = state.document().begin_create_entry(group)?;
        let draft = DraftState::new(document, DraftKind::New);
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
            if draft.needs_restore() {
                return Err(ServiceError::DraftNeedsRestore);
            }
            if draft.entry_id() == Some(id) {
                return Ok(stamped(session, draft.view()));
            }
            return Err(ServiceError::EditorAlreadyOpen);
        }
        let document = state.document().begin_edit_entry(id)?;
        let draft = DraftState::new(document, DraftKind::Existing);
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
            .draft
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
            .draft
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
        let draft = self
            .checked_mut(session)?
            .draft
            .as_mut()
            .ok_or(ServiceError::NoDraft)?;
        Ok(stamped(session, draft.update(fields)?))
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
        Ok(stamped(session, draft.set_expiry_input(input)?))
    }

    /// Confirm the form after storage succeeds; errors retain the draft and prior document.
    pub fn save_draft(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        let draft = state.draft.as_ref().ok_or(ServiceError::NoDraft)?;
        let mut candidate = state.document().clone();
        let id = draft.save(&mut candidate, now)?;
        state.commit(candidate)?;
        if let Some(file) = &state.file {
            file.discard_draft()?;
        }
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
        if let Some(file) = &state.file {
            file.discard_draft()?;
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
        .is_some_and(|draft| draft.needs_restore())
    {
        ServiceError::DraftNeedsRestore
    } else {
        ServiceError::EditorAlreadyOpen
    }
}

fn stash_draft(state: &mut DatabaseState) {
    if let Some(draft) = &mut state.draft {
        if draft.is_dirty() {
            draft.interrupt();
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
    fn conflicted_snapshots_never_leak_an_implicit_winner_into_ui_views() {
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
        service.save_draft(&session).unwrap();
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
