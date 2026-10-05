use super::{CipherWriter, ciphertext::RemoteDocument, failure, runtime};
use crate::{AndroidError, CiphertextFiles, DocumentPersistence, Host};
use std::{
    io::{Read, Write},
    sync::{Arc, Mutex},
};
use taypeer_core::{DraftId, EntryId, FieldUpdate, GroupId, OperationId, RevisionId};
use taypeer_runtime::{Command, Worker};

/// Private framed stream owned by an isolated Service, never a public document API.
#[uniffi::export(callback_interface)]
pub trait WorkerStreams: Send + Sync {
    /// Read at most 64 KiB; an empty result is EOF.
    fn read(&self, count: u32) -> Result<Vec<u8>, AndroidError>;
    /// Write at most 64 KiB and return the immediately consumed count.
    fn write(&self, bytes: Vec<u8>) -> Result<u32, AndroidError>;
}
/// Fresh platform process generation. Control methods inspect local state or
/// enqueue termination; they must never perform blocking Binder I/O.
#[uniffi::export(callback_interface)]
pub trait DocumentProcess: Send + Sync {
    /// Bind a fresh isolated Service, private pipes and its ciphertext-only host port.
    fn start(
        &self,
        writer: Arc<CipherWriter>,
        selected: Arc<super::SelectedTransfersHost>,
    ) -> Result<(), AndroidError>;
    /// Read the private response pipe, bounded by 64 KiB per callback.
    fn read(&self, count: u32) -> Result<Vec<u8>, AndroidError>;
    /// Write the private request pipe, bounded by 64 KiB per callback.
    fn write(&self, bytes: Vec<u8>) -> Result<u32, AndroidError>;
    /// OS-confirmed process exit from the local Binder-death observer.
    fn exited(&self) -> Result<bool, AndroidError>;
    /// Enqueue forceful termination independently of the command pipe.
    fn terminate(&self) -> Result<(), AndroidError>;
}
struct Reader(Arc<dyn WorkerStreams>);
struct Writer(Arc<dyn WorkerStreams>);
impl Read for Reader {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }
        let count = bytes.len().min(crate::descriptors::TRANSFER_CHUNK);
        let received = self
            .0
            .read(count as u32)
            .map_err(|_| std::io::ErrorKind::Other)?;
        if received.len() > count {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        bytes[..received.len()].copy_from_slice(&received);
        Ok(received.len())
    }
}
impl Write for Writer {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(crate::descriptors::TRANSFER_CHUNK);
        let written = self
            .0
            .write(bytes[..count].to_vec())
            .map_err(|_| std::io::ErrorKind::Other)? as usize;
        if written > count {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
struct ProcessStreams(Arc<dyn DocumentProcess>);
impl WorkerStreams for ProcessStreams {
    fn read(&self, count: u32) -> Result<Vec<u8>, AndroidError> {
        self.0.read(count)
    }
    fn write(&self, bytes: Vec<u8>) -> Result<u32, AndroidError> {
        self.0.write(bytes)
    }
}
struct Control(Arc<dyn DocumentProcess>);
impl taypeer_runtime::platform::ProcessHandle for Control {
    fn has_exited(&mut self) -> Result<bool, taypeer_runtime::RuntimeError> {
        self.0
            .exited()
            .map_err(|_| taypeer_runtime::RuntimeError::Transport)
    }
    fn terminate(&mut self) -> Result<(), taypeer_runtime::RuntimeError> {
        self.0
            .terminate()
            .map_err(|_| taypeer_runtime::RuntimeError::Transport)
    }
}
pub(super) struct Launcher {
    pub(super) process: Arc<dyn DocumentProcess>,
    pub(super) writer: Arc<CipherWriter>,
    pub(super) selected: Arc<super::SelectedTransfersHost>,
}
impl taypeer_runtime::platform::ProcessLauncher for Launcher {
    fn launch(
        &self,
    ) -> Result<taypeer_runtime::platform::ProcessConnection, taypeer_runtime::RuntimeError> {
        self.process
            .start(Arc::clone(&self.writer), Arc::clone(&self.selected))
            .map_err(|_| taypeer_runtime::RuntimeError::Transport)?;
        let streams: Arc<dyn WorkerStreams> = Arc::new(ProcessStreams(Arc::clone(&self.process)));
        Ok(taypeer_runtime::platform::ProcessConnection::new(
            Box::new(Control(Arc::clone(&self.process))),
            Box::new(Writer(Arc::clone(&streams))),
            Box::new(Reader(streams)),
        ))
    }
}
/// Serve private pipes inside DocumentService. Every temporary file comes from
/// transferred descriptor leases; no profile or app path is opened in this process.
#[uniffi::export]
pub fn serve_document(
    streams: Box<dyn WorkerStreams>,
    persistence: Box<dyn DocumentPersistence>,
    files: Box<dyn CiphertextFiles>,
    selected: Box<dyn super::SelectedTransfersRemote>,
) -> Result<(), AndroidError> {
    let streams: Arc<dyn WorkerStreams> = Arc::from(streams);
    let document = Arc::new(RemoteDocument {
        port: Arc::from(persistence),
        files: Arc::from(files),
        selected: Arc::from(selected),
        copy: Mutex::new(None),
    });
    taypeer_runtime::platform_worker::run_platform_worker(
        Reader(Arc::clone(&streams)),
        Writer(streams),
        document,
    )
    .map_err(runtime)
}

/// Public identity and path only, safe to list while locked.
#[derive(uniffi::Record)]
pub struct WorkingCopyView {
    /// Logical identity bound to the authenticated file.
    pub database: String,
    /// Pinned public trust-lineage digest.
    pub root: String,
    /// Host-owned canonical working path; isolated workers never open it.
    pub path: String,
}
impl From<taypeer_runtime::WorkingCopy> for WorkingCopyView {
    fn from(copy: taypeer_runtime::WorkingCopy) -> Self {
        Self {
            database: copy.database.to_string(),
            root: copy.root.to_string(),
            path: copy.path.to_string_lossy().into_owned(),
        }
    }
}
#[uniffi::export]
impl Host {
    /// Read the common durable catalog without opening plaintext documents.
    pub fn working_copies(&self) -> Result<Vec<WorkingCopyView>, AndroidError> {
        self.runtime
            .working_copies()
            .map(|copies| copies.into_iter().map(Into::into).collect())
            .map_err(runtime)
    }
    /// Copy a read-only SAF staging file into the common internal working directory.
    /// Catalog publication follows successful worker authentication.
    pub fn stage_import(&self, source: String) -> Result<WorkingCopyView, AndroidError> {
        self.runtime
            .stage_external_copy(std::path::Path::new(&source))
            .map(Into::into)
            .map_err(runtime)
    }
    /// Open a catalog item in a fresh isolated process, off the UI thread.
    pub fn open_document(
        &self,
        path: String,
        password: String,
        process: Box<dyn DocumentProcess>,
        files: Box<dyn CiphertextFiles>,
    ) -> Result<Arc<DocumentSession>, AndroidError> {
        let copy = self
            .runtime
            .stage_external_copy(std::path::Path::new(&path))
            .map_err(runtime)?;
        self.open_session(copy.path, password, None, process, files)
    }
    /// Create internally; no directory picker is involved and retries retain the path.
    pub fn create_document(
        &self,
        name: String,
        description: Option<String>,
        operation: String,
        password: String,
        process: Box<dyn DocumentProcess>,
        files: Box<dyn CiphertextFiles>,
    ) -> Result<Arc<DocumentSession>, AndroidError> {
        let path = self
            .runtime
            .creation_path(&OperationId::new(identifier(operation)?))
            .map_err(runtime)?;
        let form = Some(taypeer_services::CreateDatabase {
            name,
            description,
            policy: Default::default(),
        });
        self.open_session(path, password, form, process, files)
    }
    /// Start direct exchange explicitly. Ciphertext reception remains active while locked.
    pub fn start_exchange(&self) -> Result<(), AndroidError> {
        self.runtime
            .start_network(taypeer_sync::RelaySetting::Disabled)
            .map(|_| ())
            .map_err(runtime)
    }
    /// Stop network lifetime without unlocking any file.
    pub fn stop_exchange(&self) {
        self.runtime.stop_network();
    }
}
impl Host {
    fn open_session(
        &self,
        path: std::path::PathBuf,
        password: String,
        form: Option<taypeer_services::CreateDatabase>,
        process: Box<dyn DocumentProcess>,
        files: Box<dyn CiphertextFiles>,
    ) -> Result<Arc<DocumentSession>, AndroidError> {
        let writer = Arc::new(CipherWriter {
            writer: self
                .runtime
                .platform_cipher_writer(&path)
                .map_err(runtime)?,
            files: Arc::from(files),
        });
        let launcher = Launcher {
            writer,
            process: Arc::from(process),
            selected: Arc::new(super::SelectedTransfersHost::default()),
        };
        let retry = if path.try_exists().map_err(failure)? {
            form.clone()
        } else {
            None
        };
        let mut worker = self
            .runtime
            .open_platform_worker(
                &launcher,
                password,
                if retry.is_some() { None } else { form },
            )
            .map_err(runtime)?;
        if let Some(form) = retry {
            taypeer_runtime::RuntimeHost::validate_creation_retry(&mut worker, &form)
                .map_err(runtime)?;
        }
        if let Err(error) = self.runtime.register_working_copy(&path) {
            worker.invalidate(taypeer_runtime::session::LockReason::Transport);
            return Err(runtime(error));
        }
        let control = worker.control();
        launcher.selected.bind(control.clone())?;
        Ok(Arc::new(DocumentSession {
            worker: Mutex::new(worker),
            control,
            host: Arc::clone(&self.runtime),
            selected: launcher.selected,
        }))
    }
}

/// Keep, set and clear retain distinct meaning; an empty Set is never silently normalized.
#[derive(uniffi::Enum)]
pub enum TextEdit {
    /// Leave the existing value untouched.
    Keep,
    /// Set the exact submitted value, including an empty string.
    Set {
        /// Exact submitted input.
        value: String,
    },
    /// Remove the optional value.
    Clear,
}
impl TextEdit {
    fn update(self) -> Result<FieldUpdate<String>, AndroidError> {
        Ok(match self {
            Self::Keep => FieldUpdate::Keep,
            Self::Clear => FieldUpdate::Clear,
            Self::Set { value } => {
                if value.len() > 256 * 1024 {
                    return Err(AndroidError::InvalidOptions);
                }
                FieldUpdate::Set(value)
            }
        })
    }
}
/// One explicitly addressed entry field.
#[derive(uniffi::Enum)]
pub enum EntryTextField {
    /// Required entry label.
    Title,
    /// Optional login.
    Username,
    /// Protected password value.
    Password,
    /// Optional URL.
    Url,
    /// Optional atomic notes.
    Notes,
}
/// Descriptive form category. Business rules stay in the common Rust service.
#[derive(Clone, Copy, uniffi::Enum)]
pub enum FormKind {
    /// Existing entry form.
    Entry,
    /// Independent unfinished new entry.
    NewEntry,
    /// Existing group metadata.
    Group,
    /// Independent unfinished group.
    NewGroup,
    /// Database name and description.
    Database,
}
/// A local editor identity, deliberately excluding its entered values.
#[derive(uniffi::Record)]
pub struct LocalForm {
    /// Stable local form identity.
    pub draft: String,
    /// Exact local input revision, independent of history timestamps.
    pub revision: u64,
    /// Typed form category.
    pub kind: FormKind,
    /// Existing or reserved object identity, absent for database metadata.
    pub target: Option<String>,
    /// Whether local input differs from its confirmed baseline.
    pub dirty: bool,
}
fn form(identity: taypeer_services::DraftIdentity, dirty: bool) -> LocalForm {
    use taypeer_services::DraftTarget;
    let (kind, target) = match identity.target {
        DraftTarget::Entry(id) => (FormKind::Entry, Some(id.to_string())),
        DraftTarget::NewEntry { entry, .. } => (FormKind::NewEntry, Some(entry.to_string())),
        DraftTarget::Group(id) => (FormKind::Group, Some(id.to_string())),
        DraftTarget::NewGroup { group, .. } => (FormKind::NewGroup, Some(group.to_string())),
        DraftTarget::Database => (FormKind::Database, None),
    };
    LocalForm {
        draft: identity.draft.to_string(),
        revision: identity.revision.0,
        kind,
        target,
        dirty,
    }
}
/// Masked stable attribute row; protected values are absent.
#[derive(uniffi::Record)]
pub struct AttributeRow {
    /// Stable identity of the addressed item.
    pub id: Option<String>,
    /// Exact ordinary descriptive name.
    pub name: String,
    /// Ordinary value; absent when protected.
    pub value: Option<String>,
    /// Whether explicit reveal is required.
    pub protected: bool,
}
/// Masked current editor, addressed to one database and process generation.
#[derive(uniffi::Record)]
pub struct EntryEditor {
    /// Logical identity bound to the authenticated file.
    pub database: String,
    /// Process generation used to reject late responses.
    pub generation: u64,
    /// Exact local draft identity and input revision.
    pub form: LocalForm,
    /// Existing entry identity; absent before the first confirmed save.
    pub entry: Option<String>,
    /// Selected group identity; absent means ungrouped.
    pub group: Option<String>,
    /// Exact ordinary entry title.
    pub title: String,
    /// Optional ordinary login.
    pub username: Option<String>,
    /// Optional ordinary URL; reading does not access the network.
    pub url: Option<String>,
    /// Optional ordinary notes.
    pub notes: Option<String>,
    /// Atomic tag set.
    pub tags: Vec<String>,
    /// Whether a password exists; its value is excluded.
    pub has_password: bool,
    /// Selected saved expiration, retained when the raw input is not being edited.
    pub expires_at: Option<i64>,
    /// Unfinished expiration input retained by the local form.
    pub expiry_input: Option<String>,
    /// Masked attributes retaining independent identities.
    pub attributes: Vec<AttributeRow>,
}
/// Saved entry projection. Password and protected attributes require an explicit reveal.
#[derive(uniffi::Record)]
pub struct EntryRow {
    /// Stable identity of the addressed item.
    pub id: String,
    /// Selected group identity; absent means ungrouped.
    pub group: Option<String>,
    /// Exact ordinary entry title.
    pub title: String,
    /// Optional ordinary login.
    pub username: Option<String>,
    /// Optional ordinary URL; reading does not access the network.
    pub url: Option<String>,
    /// Optional ordinary notes.
    pub notes: Option<String>,
    /// Presence when explicitly inspected; list summaries omit this fact.
    pub has_password: Option<bool>,
    /// Whether original alternative values remain in history.
    pub alternatives: bool,
}
fn entry_row(view: taypeer_services::EntryView) -> EntryRow {
    EntryRow {
        id: view.id.to_string(),
        group: view.group_id.map(|id| id.to_string()),
        title: view.title,
        username: view.username,
        url: view.url,
        notes: view.notes,
        has_password: Some(view.has_password),
        alternatives: view.has_conflicts,
    }
}
/// Active group metadata; no decrypted document tree is exported.
#[derive(uniffi::Record)]
pub struct GroupRow {
    /// Stable identity of the addressed item.
    pub id: String,
    /// Selected parent; absent means the tree root.
    pub parent: Option<String>,
    /// Exact ordinary descriptive name.
    pub name: String,
    /// Exact optional ordinary descriptive text.
    pub description: Option<String>,
}
/// Current selected catalog values after authentication.
#[derive(uniffi::Record)]
pub struct DocumentOverview {
    /// Logical identity bound to the authenticated file.
    pub database: String,
    /// Process generation used to reject late responses.
    pub generation: u64,
    /// Exact ordinary descriptive name.
    pub name: String,
    /// Exact optional ordinary descriptive text.
    pub description: Option<String>,
    /// Whether the authenticated author may change this database.
    pub writable: bool,
    /// Whether this device manages administrative policy.
    pub managing: bool,
    /// Selected active groups.
    pub groups: Vec<GroupRow>,
    /// Selected masked entry rows.
    pub entries: Vec<EntryRow>,
}
/// Ordinary metadata input with its exact local identity.
#[derive(uniffi::Record)]
pub struct MetadataEditor {
    /// Exact local draft identity and input revision.
    pub form: LocalForm,
    /// Exact ordinary descriptive name.
    pub name: String,
    /// Exact optional ordinary descriptive text.
    pub description: Option<String>,
}
/// Confirmed content version; entry secrets stay masked until explicit reveal.
#[derive(uniffi::Record)]
pub struct HistoryRow {
    /// Stable identity of the addressed item.
    pub id: String,
    /// Exact ordinary descriptive name.
    pub name: String,
    /// Exact optional ordinary descriptive text.
    pub description: Option<String>,
    /// Display timestamp in UTC milliseconds; never used as merge rank.
    pub saved_at: i64,
}
/// Durability of the captured form snapshot.
#[derive(uniffi::Enum)]
pub enum SaveState {
    /// Database content and history were durably committed.
    Saved,
    /// Incomplete input was durably retained locally.
    LocalDraftSaved,
    /// No effective input change required a database version.
    Unchanged,
}
/// Acknowledgement of an exact input revision; later input is never replaced by it.
#[derive(uniffi::Record)]
pub struct SaveResult {
    /// Logical identity bound to the authenticated file.
    pub database: String,
    /// Process generation used to reject late responses.
    pub generation: u64,
    /// Exact local draft identity and input revision.
    pub form: LocalForm,
    /// Stable operation identity reused after a lost response.
    pub operation: String,
    /// Durability of the exact captured snapshot.
    pub state: SaveState,
}

/// Nonblocking access phase from the independent supervisor.
#[derive(uniffi::Enum)]
pub enum SessionAccess {
    /// Authenticated access is currently permitted.
    Open,
    /// Access is revoked; the process may still be shutting down.
    Revoked,
}
/// One supervised isolated document. Lock control is separate from the command mutex.
#[derive(uniffi::Object)]
pub struct DocumentSession {
    worker: Mutex<Worker>,
    control: taypeer_runtime::WorkerControl,
    pub(super) host: Arc<taypeer_runtime::RuntimeHost>,
    pub(super) selected: Arc<super::SelectedTransfersHost>,
}
impl DocumentSession {
    pub(super) fn command<T: serde::de::DeserializeOwned>(
        &self,
        mut command: Command,
    ) -> Result<T, AndroidError> {
        let mut worker = self.worker.lock().map_err(failure)?;
        let result = worker.request(&command);
        command.erase_input();
        let value = result.map_err(runtime)?;
        if !worker.is_open() {
            return Err(AndroidError::Runtime);
        }
        let decoded = serde_json::from_value(value).map_err(failure)?;
        if !worker.is_open() {
            return Err(AndroidError::Runtime);
        }
        Ok(decoded)
    }
    fn identity(&self) -> Result<(String, u64), AndroidError> {
        let status = self.control.status();
        if status.phase != taypeer_runtime::session::SessionPhase::Open {
            return Err(AndroidError::Runtime);
        }
        Ok((
            status.database.ok_or(AndroidError::Runtime)?.to_string(),
            status.generation,
        ))
    }
}
#[uniffi::export]
impl DocumentSession {
    /// Check access independently of a busy command or Binder operation.
    pub fn access(&self) -> SessionAccess {
        if self.control.is_open() {
            SessionAccess::Open
        } else {
            SessionAccess::Revoked
        }
    }
    /// Nonsecret process generation used to reject late client results.
    pub fn generation(&self) -> u64 {
        self.control.status().generation
    }
    /// Current logical identity after authenticated boot.
    pub fn database_id(&self) -> Result<String, AndroidError> {
        Ok(self.identity()?.0)
    }
    /// Immediate access revocation; completion does not wait for a busy command.
    pub fn lock(&self) {
        self.selected.revoke();
        self.control
            .invalidate(taypeer_runtime::session::LockReason::Manual);
    }
    /// Confirm process exit and draft disposition with the shared bounded deadline.
    pub fn close_session(&self) -> Result<(), AndroidError> {
        self.worker
            .lock()
            .map_err(failure)?
            .close()
            .map_err(runtime)
    }
    /// Selected values and groups; a missing group stays ungrouped.
    pub fn overview(&self, query: String) -> Result<DocumentOverview, AndroidError> {
        if query.len() > 65536 {
            return Err(AndroidError::InvalidOptions);
        }
        let info: taypeer_services::DatabaseInfo = self.command(Command::DatabaseInfo)?;
        let groups: Vec<taypeer_services::GroupInfo> = self.command(Command::GroupInfo)?;
        let entries: Vec<taypeer_services::EntrySummary> =
            self.command(Command::Entries { group: None, query })?;
        let (database, generation) = self.identity()?;
        Ok(DocumentOverview {
            database,
            generation,
            name: info.name,
            description: info.description,
            writable: info.writable,
            managing: info.managing,
            groups: groups
                .into_iter()
                .map(|g| GroupRow {
                    id: g.group.id.to_string(),
                    parent: g.group.parent.map(|id| id.to_string()),
                    name: g.group.name,
                    description: g.description,
                })
                .collect(),
            entries: entries
                .into_iter()
                .map(|e| EntryRow {
                    id: e.id.to_string(),
                    group: e.group_id.map(|id| id.to_string()),
                    title: e.title,
                    username: e.username,
                    url: e.url,
                    notes: e.notes,
                    has_password: None,
                    alternatives: e.has_conflicts,
                })
                .collect(),
        })
    }
    /// Current masked saved entry.
    pub fn entry(&self, id: String) -> Result<EntryRow, AndroidError> {
        self.command(Command::Entry(EntryId::new(identifier(id)?)))
            .map(entry_row)
    }
    /// Begin or automatically continue an existing entry; absence creates an independent form.
    pub fn begin_entry(
        &self,
        entry: Option<String>,
        group: Option<String>,
    ) -> Result<EntryEditor, AndroidError> {
        let command = match entry {
            Some(id) => Command::BeginEdit(EntryId::new(identifier(id)?)),
            None => match group {
                Some(id) => Command::BeginCreate(GroupId::new(identifier(id)?)),
                None => Command::BeginCreateUngrouped,
            },
        };
        self.command::<serde_json::Value>(command)?;
        self.editor()
    }
    /// Masked editor values; unchanged passwords are never copied into input fields.
    pub fn editor(&self) -> Result<EntryEditor, AndroidError> {
        let view: taypeer_services::EditorView = self.command(Command::EditorView)?;
        let (database, generation) = self.identity()?;
        Ok(EntryEditor {
            database,
            generation,
            form: form(view.identity, view.dirty),
            entry: view.entry.map(|id| id.to_string()),
            group: view.group.map(|id| id.to_string()),
            title: view.fields.title,
            username: view.fields.username,
            url: view.fields.url,
            notes: view.fields.notes,
            tags: view.fields.tags,
            has_password: view.has_password,
            expires_at: view.fields.expires_at,
            expiry_input: view.expiry_input,
            attributes: view
                .fields
                .attributes
                .into_iter()
                .map(|a| AttributeRow {
                    id: a.id.map(|id| id.to_string()),
                    name: a.name,
                    value: (!a.protected).then_some(a.value),
                    protected: a.protected,
                })
                .collect(),
        })
    }
    /// Submit one addressed field, preserving all omitted values and secret masks.
    pub fn patch_entry(
        &self,
        field: EntryTextField,
        edit: TextEdit,
    ) -> Result<EntryEditor, AndroidError> {
        let mut patch = taypeer_services::EntryPatch::default();
        let update = edit.update()?;
        match field {
            EntryTextField::Title => patch.title = update,
            EntryTextField::Username => patch.username = update,
            EntryTextField::Password => patch.password = update,
            EntryTextField::Url => patch.url = update,
            EntryTextField::Notes => patch.notes = update,
        }
        self.command::<serde_json::Value>(Command::PatchDraft(patch))?;
        self.editor()
    }
    /// Edit one atomic attribute, or remove precisely its identity.
    pub fn patch_attribute(
        &self,
        id: Option<String>,
        name: String,
        value: TextEdit,
        protected: bool,
        remove: bool,
    ) -> Result<EntryEditor, AndroidError> {
        if name.len() > 65536 {
            return Err(AndroidError::InvalidOptions);
        }
        self.command::<serde_json::Value>(Command::PatchAttribute {
            patch: taypeer_services::AttributePatch {
                id: id
                    .map(identifier)
                    .transpose()?
                    .map(taypeer_core::AttributeId::new),
                name,
                value: value.update()?,
                protected,
            },
            remove,
        })?;
        self.editor()
    }
    /// Preserve unfinished expiration input in the encrypted form.
    pub fn patch_expiry(&self, input: Option<String>) -> Result<EntryEditor, AndroidError> {
        if input.as_ref().is_some_and(|value| value.len() > 65536) {
            return Err(AndroidError::InvalidOptions);
        }
        let selected = match &input {
            None => Some(FieldUpdate::Clear),
            Some(value) => value.parse::<i64>().ok().map(FieldUpdate::Set),
        };
        if let Some(expires_at) = selected {
            self.command::<serde_json::Value>(Command::PatchDraft(taypeer_services::EntryPatch {
                expires_at,
                ..Default::default()
            }))?;
            self.command::<serde_json::Value>(Command::DraftExpiry(None))?;
        } else {
            self.command::<serde_json::Value>(Command::DraftExpiry(input))?;
        }
        self.editor()
    }
    /// Save the exact snapshot without closing its editor; reuse operation after lost response.
    pub fn save(
        &self,
        draft: String,
        revision: u64,
        operation: String,
    ) -> Result<SaveResult, AndroidError> {
        let result: taypeer_services::DraftSaveOutcome =
            self.command(Command::SaveDraftSnapshot {
                draft: DraftId::new(identifier(draft)?),
                revision: taypeer_services::DraftRevision(revision),
                operation: OperationId::new(identifier(operation)?),
            })?;
        use taypeer_services::DraftSaveOutcome;
        let (identity, operation, state) = match result {
            DraftSaveOutcome::Saved {
                identity,
                operation,
            } => (identity, operation, SaveState::Saved),
            DraftSaveOutcome::LocalDraftSaved {
                identity,
                operation,
            } => (identity, operation, SaveState::LocalDraftSaved),
            DraftSaveOutcome::Unchanged {
                identity,
                operation,
            } => (identity, operation, SaveState::Unchanged),
        };
        let (database, generation) = self.identity()?;
        Ok(SaveResult {
            database,
            generation,
            form: form(identity, false),
            operation: operation.to_string(),
            state,
        })
    }
    /// Local encrypted fallback for navigation after a document write failure.
    pub fn persist_forms(&self) -> Result<(), AndroidError> {
        self.command::<serde_json::Value>(Command::PersistDrafts)
            .map(|_| ())
    }
    /// Durably retain the exact active form through an OS picker, including a clean baseline.
    pub fn pin_active_form(&self) -> Result<LocalForm, AndroidError> {
        let identity: taypeer_services::DraftIdentity = self.command(Command::PinActiveForm)?;
        self.forms()?
            .into_iter()
            .find(|form| form.draft == identity.draft.as_str())
            .ok_or(AndroidError::Runtime)
    }
    /// Release only the selected OS picker pin after fresh authentication or cancellation.
    pub fn unpin_form(&self, draft: String) -> Result<(), AndroidError> {
        self.command::<serde_json::Value>(Command::UnpinForm(DraftId::new(identifier(draft)?)))?;
        Ok(())
    }
    /// Independent form identities, without entered labels or secret values.
    pub fn forms(&self) -> Result<Vec<LocalForm>, AndroidError> {
        self.command::<Vec<taypeer_services::DraftSummary>>(Command::Drafts)
            .map(|forms| {
                forms
                    .into_iter()
                    .map(|f| form(f.identity, f.dirty))
                    .collect()
            })
    }
    /// Exact most recently active form; listing order never substitutes for identity.
    pub fn active_form(&self) -> Result<Option<LocalForm>, AndroidError> {
        self.command::<Option<taypeer_services::DraftIdentity>>(Command::ActiveDraft)
            .map(|identity| identity.map(|identity| form(identity, true)))
    }
    /// Activate a retained form in this generation.
    pub fn resume_form(&self, draft: String) -> Result<(), AndroidError> {
        self.command::<serde_json::Value>(Command::ResumeDraft(DraftId::new(identifier(draft)?)))
            .map(|_| ())
    }
    /// Explicitly discard only this local form.
    pub fn delete_form(&self, draft: String) -> Result<(), AndroidError> {
        self.command::<serde_json::Value>(Command::DeleteDraft(DraftId::new(identifier(draft)?)))
            .map(|_| ())
    }
    /// Begin/continue descriptive metadata; missing group creates an independent root group.
    pub fn begin_metadata(
        &self,
        kind: FormKind,
        group: Option<String>,
        parent: Option<String>,
    ) -> Result<MetadataEditor, AndroidError> {
        let command = match kind {
            FormKind::Database => Command::BeginEditDatabaseInfo,
            FormKind::Group => Command::BeginEditGroup(GroupId::new(identifier(
                group.ok_or(AndroidError::InvalidOptions)?,
            )?)),
            FormKind::NewGroup => {
                Command::BeginCreateGroup(parent.map(identifier).transpose()?.map(GroupId::new))
            }
            _ => return Err(AndroidError::InvalidOptions),
        };
        let view: taypeer_services::MetadataDraftView = self.command(command)?;
        Ok(MetadataEditor {
            form: form(view.identity, view.dirty),
            name: view.name,
            description: view.description,
        })
    }
    /// Read a metadata form after resume without replacing its identity.
    pub fn metadata(&self, draft: String) -> Result<MetadataEditor, AndroidError> {
        let view: taypeer_services::MetadataDraftView =
            self.command(Command::MetadataDraft(DraftId::new(identifier(draft)?)))?;
        Ok(MetadataEditor {
            form: form(view.identity, view.dirty),
            name: view.name,
            description: view.description,
        })
    }
    /// Addressed name/description changes retain their original causal context.
    pub fn patch_metadata(
        &self,
        draft: String,
        kind: FormKind,
        name: TextEdit,
        description: TextEdit,
    ) -> Result<MetadataEditor, AndroidError> {
        let id = DraftId::new(identifier(draft.clone())?);
        let name = name.update()?;
        let description = description.update()?;
        let command = match kind {
            FormKind::Database => Command::PatchDatabaseDraft {
                draft: id,
                patch: taypeer_core::DatabaseMetadataPatch { name, description },
            },
            FormKind::Group | FormKind::NewGroup => Command::PatchGroupDraft {
                draft: id,
                patch: taypeer_core::GroupMetadataPatch {
                    name,
                    description,
                    icon: FieldUpdate::Keep,
                },
            },
            _ => return Err(AndroidError::InvalidOptions),
        };
        self.command::<serde_json::Value>(command)?;
        self.metadata(draft)
    }
    /// Original saved versions; automatic merges and retries create no rows.
    pub fn history(
        &self,
        kind: FormKind,
        target: Option<String>,
    ) -> Result<Vec<HistoryRow>, AndroidError> {
        Ok(match kind {
            FormKind::Entry => self
                .command::<Vec<taypeer_services::RevisionSummary>>(Command::History(EntryId::new(
                    identifier(target.ok_or(AndroidError::InvalidOptions)?)?,
                )))?
                .into_iter()
                .map(|r| HistoryRow {
                    id: r.id.to_string(),
                    name: r.title,
                    description: None,
                    saved_at: r.saved_at,
                })
                .collect(),
            FormKind::Group => self
                .command::<Vec<taypeer_core::SavedGroupRevision>>(Command::GroupHistory(
                    GroupId::new(identifier(target.ok_or(AndroidError::InvalidOptions)?)?),
                ))?
                .into_iter()
                .map(|r| HistoryRow {
                    id: r.id.to_string(),
                    name: r.snapshot.group.name,
                    description: r.snapshot.description,
                    saved_at: r.saved_at,
                })
                .collect(),
            FormKind::Database => self
                .command::<Vec<taypeer_core::SavedDatabaseRevision>>(Command::DatabaseHistory)?
                .into_iter()
                .map(|r| HistoryRow {
                    id: r.id.to_string(),
                    name: r.snapshot.name,
                    description: r.snapshot.description,
                    saved_at: r.saved_at,
                })
                .collect(),
            _ => return Err(AndroidError::InvalidOptions),
        })
    }
    /// Read a selected historical entry without protected values.
    pub fn revision(&self, entry: String, revision: String) -> Result<EntryRow, AndroidError> {
        self.command(Command::Revision {
            entry: EntryId::new(identifier(entry)?),
            revision: RevisionId::new(identifier(revision)?),
        })
        .map(entry_row)
    }
    /// Reveal only on the user's explicit action; client clears the returned JVM string on lock.
    pub fn reveal(
        &self,
        entry: String,
        revision: Option<String>,
        attribute: Option<String>,
    ) -> Result<String, AndroidError> {
        let entry = EntryId::new(identifier(entry)?);
        let command = match revision {
            Some(revision) => Command::RevealRevision {
                entry,
                revision: RevisionId::new(identifier(revision)?),
                attribute: attribute
                    .map(identifier)
                    .transpose()?
                    .map(taypeer_core::AttributeId::new),
            },
            None => match attribute {
                Some(attribute) => Command::RevealAttribute {
                    entry,
                    attribute: taypeer_core::AttributeId::new(identifier(attribute)?),
                },
                None => Command::RevealPassword(entry),
            },
        };
        self.command(command)
    }
    /// Trash immediately without a confirmation dialog; caller offers Undo.
    pub fn trash(
        &self,
        kind: FormKind,
        target: String,
        operation: String,
    ) -> Result<(), AndroidError> {
        let target = match kind {
            FormKind::Entry => taypeer_services::ObjectId::Entry(EntryId::new(identifier(target)?)),
            FormKind::Group => taypeer_services::ObjectId::Group(GroupId::new(identifier(target)?)),
            _ => return Err(AndroidError::InvalidOptions),
        };
        self.command::<serde_json::Value>(Command::TrashObject {
            target,
            operation: OperationId::new(identifier(operation)?),
        })
        .map(|_| ())
    }
    /// Return the selected trash object to an explicit group or the ungrouped/root location.
    pub fn undo_trash(
        &self,
        kind: FormKind,
        target: String,
        group: Option<String>,
        operation: String,
    ) -> Result<(), AndroidError> {
        let target = match kind {
            FormKind::Entry => taypeer_services::ObjectId::Entry(EntryId::new(identifier(target)?)),
            FormKind::Group => taypeer_services::ObjectId::Group(GroupId::new(identifier(target)?)),
            _ => return Err(AndroidError::InvalidOptions),
        };
        let prepared = self.command(Command::PrepareLifecycle {
            action: taypeer_services::LifecycleAction::Restore,
            target,
            destination: group.map(identifier).transpose()?.map(GroupId::new),
        })?;
        self.command::<serde_json::Value>(Command::ConfirmLifecycle {
            prepared,
            operation: OperationId::new(identifier(operation)?),
        })
        .map(|_| ())
    }
    /// Explicit purge of precisely reviewed history; UI retains its confirmation.
    pub fn purge_history(
        &self,
        kind: FormKind,
        target: Option<String>,
        revisions: Vec<String>,
        operation: String,
    ) -> Result<(), AndroidError> {
        if revisions.len() > 4096 {
            return Err(AndroidError::InvalidOptions);
        }
        let revisions = revisions
            .into_iter()
            .map(identifier)
            .map(|id| id.map(RevisionId::new))
            .collect::<Result<Vec<_>, _>>()?;
        let operation = OperationId::new(identifier(operation)?);
        let command = match kind {
            FormKind::Entry => Command::PurgeHistory {
                entry: EntryId::new(identifier(target.ok_or(AndroidError::InvalidOptions)?)?),
                revisions: revisions.into_iter().collect(),
                operation,
            },
            FormKind::Group => Command::PurgeGroupHistory {
                group: GroupId::new(identifier(target.ok_or(AndroidError::InvalidOptions)?)?),
                revisions,
                operation,
            },
            FormKind::Database => Command::PurgeDatabaseHistory {
                revisions,
                operation,
            },
            _ => return Err(AndroidError::InvalidOptions),
        };
        self.command::<serde_json::Value>(command).map(|_| ())
    }
}
pub(super) fn identifier(value: String) -> Result<String, AndroidError> {
    if value.is_empty() || value.len() > 256 || value.chars().any(char::is_control) {
        Err(AndroidError::InvalidOptions)
    } else {
        Ok(value)
    }
}
