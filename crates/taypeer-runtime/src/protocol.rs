//! Private, length-prefixed JSON messages. This is not the network/file protocol.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use taypeer_core::{AttributeId, DraftId, EntryId, GroupId, OperationId, RevisionId};
use taypeer_services::{
    ConflictContext, EntryPatch, GroupMove, InspectionTarget, LifecycleAction, ObjectAddress,
    ObjectId, PreparedLifecycle, RecoveryRequest, Resolution, ServiceError,
};
use zeroize::{Zeroize, Zeroizing};

pub(crate) const MAX_MESSAGE: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(crate) struct Boot {
    pub path: PathBuf,
    pub password: String,
    pub create_name: Option<String>,
    #[serde(default)]
    pub create_form: Option<taypeer_services::CreateDatabase>,
    pub profile: PathBuf,
    pub spool: PathBuf,
    pub invitation: Option<taypeer_trust::Invitation>,
}

impl Drop for Boot {
    fn drop(&mut self) {
        self.password.zeroize();
    }
}

/// Commands dispatched to the same session-checked Rust services as the GUI.
/// No command implicitly reveals a protected value.
#[derive(Serialize, Deserialize)]
pub enum Command {
    /// Read authenticated descriptive metadata.
    DatabaseInfo,
    /// Confirm descriptive fields in a single durable transaction.
    SetDatabaseInfo {
        /// Stable identity for retries of this exact request.
        operation: OperationId,
        /// Exact name.
        name: String,
        /// Exact optional description.
        description: Option<String>,
    },
    /// Read detailed group forms.
    GroupInfo,
    /// Confirm the complete group form once.
    SaveGroup {
        /// Complete form.
        form: taypeer_services::GroupForm,
        /// Stable retry identity.
        operation: OperationId,
    },
    /// Read the masked active editor including restored input.
    EditorView,
    /// List independent local forms without entered names or secrets.
    Drafts,
    /// Identity of the active entry or metadata form, without its values.
    ActiveDraft,
    /// Activate a retained form; read its masked view separately.
    ResumeDraft(DraftId),
    /// Durably discard one local form, leaving its confirmed object intact.
    DeleteDraft(DraftId),
    /// Confirm a durable local fallback for navigation after a document write failure.
    /// This never reports the database as saved or creates a history revision.
    PersistDrafts,
    /// Explicitly retain the active form during an OS picker background transition.
    /// Clean forms retain their identity without a document or input revision.
    PinActiveForm,
    /// Durably release only the specified OS picker pin.
    UnpinForm(DraftId),
    /// Durably save the exact captured revision, preserving the editor and later input.
    SaveDraftSnapshot {
        /// Stable local editor identity.
        draft: DraftId,
        /// Exact input revision.
        revision: taypeer_services::DraftRevision,
        /// Same identity must be reused when retrying this snapshot.
        operation: OperationId,
    },
    /// Begin an unfinished entry outside every group.
    BeginCreateUngrouped,
    /// Begin or continue a group's local form.
    BeginEditGroup(GroupId),
    /// Begin an unfinished group in the selected parent, or at the root.
    BeginCreateGroup(Option<GroupId>),
    /// Begin or continue database descriptive metadata.
    BeginEditDatabaseInfo,
    /// Read retained versions of the selected group, including original alternatives.
    GroupHistory(GroupId),
    /// Read retained versions of database descriptive metadata.
    DatabaseHistory,
    /// Explicitly purge exactly the reviewed group revisions.
    PurgeGroupHistory {
        /// Selected group identity.
        group: GroupId,
        /// Exact reviewed versions.
        revisions: Vec<RevisionId>,
        /// Stable retry identity.
        operation: OperationId,
    },
    /// Explicitly purge exactly the reviewed database metadata revisions.
    PurgeDatabaseHistory {
        /// Exact reviewed versions.
        revisions: Vec<RevisionId>,
        /// Stable retry identity.
        operation: OperationId,
    },
    /// Read descriptive form input in the current authenticated session.
    MetadataDraft(DraftId),
    /// Addressed group input, preserving original causal heads.
    PatchGroupDraft {
        /// Exact local form.
        draft: DraftId,
        /// Keep/set/clear fields.
        patch: taypeer_core::GroupMetadataPatch,
    },
    /// Addressed database input, preserving original causal heads.
    PatchDatabaseDraft {
        /// Exact local form.
        draft: DraftId,
        /// Keep/set/clear fields.
        patch: taypeer_core::DatabaseMetadataPatch,
    },
    /// Address one attribute without revealing unchanged secret values.
    PatchAttribute {
        /// Addressed fields.
        patch: taypeer_services::AttributePatch,
        /// Remove the exact identity.
        remove: bool,
    },
    /// Preserve unfinished expiration input in the service draft.
    DraftExpiry(Option<String>),
    /// Explicitly reveal an active editor secret.
    RevealEditor(Option<AttributeId>),
    /// Explicitly reveal a historical password or attribute.
    RevealRevision {
        /// Target entry.
        entry: EntryId,
        /// Saved version.
        revision: RevisionId,
        /// Attribute, or password when absent.
        attribute: Option<AttributeId>,
    },
    /// Read the unlocked session's authenticated format compatibility.
    Compatibility,
    /// Create a separate trust set without modifying the readable original.
    RecoverTrust {
        /// New destination; existing unrelated files are never replaced.
        path: PathBuf,
        /// Explicit retry identity.
        operation: taypeer_trust::Digest,
        /// New password, confined to the plaintext worker.
        password: Zeroizing<Vec<u8>>,
    },
    /// Validate original provenance/dependencies and durably apply independently eligible packets.
    ApplyReceived,
    /// List original unaccepted sources without contents.
    ReceivedSources,
    /// Collect ciphertext after verifying all retention roots.
    CollectReceived,
    /// Inspect a selected causal state with protected fields masked.
    InspectReceived(String),
    /// Explicitly reveal a selected source password.
    RevealReceived {
        /// Exact original change hash.
        change: String,
        /// Selected source entry.
        entry: EntryId,
    },
    /// Explicitly discard an original source across all ciphertext packaging.
    DiscardReceived(String),
    /// Create a new author confirmation from a selected source entry.
    ExtractReceived {
        /// Original change hash.
        change: String,
        /// Selected entry at that causal state.
        entry: EntryId,
        /// Current destination group.
        group: Option<GroupId>,
        /// Durable exact-intent retry identity.
        operation: OperationId,
    },
    /// Read public signed device/control state, without acquiring another credential.
    Authority,
    /// Read the authority actually loaded by this worker, without refreshing host storage.
    /// Used to reconcile startup and missed network events against the exact opened epoch.
    SessionAuthority,
    /// Explicitly create and reveal one invitation's bearer material.
    CreateInvitation,
    /// Approve the recipient of a durable request.
    ApproveInvitation(taypeer_trust::Digest),
    /// Explicitly refuse a received request or cancel an unused invitation.
    CloseInvitation {
        /// Request identity.
        request: taypeer_trust::Digest,
        /// True refuses a recipient; false cancels an issued code.
        reject: bool,
    },
    /// Create signed recipient consent for an exact handoff operation.
    ConsentManagement(taypeer_trust::Digest),
    /// Durably relinquish management using the recipient's explicit signed consent.
    TransferManagement(taypeer_trust::HandoffConsent),
    /// Private enrollment command: authenticate received ciphertext using a retry password.
    /// The staging password and author remain in the enrollment worker.
    BindInvitation {
        /// Fully downloaded working file.
        path: std::path::PathBuf,
        /// Exact database master password, permitting correction after receipt.
        password: Zeroizing<Vec<u8>>,
    },
    /// Rotate password/key, optionally revoking a member in the same durable transition.
    RotatePassword {
        /// Idempotent administrative operation.
        operation: taypeer_trust::Digest,
        /// Exact explicit password input, never argv.
        password: Zeroizing<Vec<u8>>,
        /// Optional excluded device.
        revoke: Option<taypeer_trust::DeviceId>,
    },
    /// Read authenticated shared quota and KDF settings.
    DatabasePolicy,
    /// Change manager-controlled policy; password is required when recalibrating KDF.
    SetDatabasePolicy {
        /// Idempotent operation.
        operation: taypeer_trust::Digest,
        /// Validated shared settings.
        policy: taypeer_core::DatabasePolicy,
        /// Exact reauthentication input when needed.
        password: Option<Zeroizing<Vec<u8>>>,
    },
    /// Read binary-only metadata from an explicit scope.
    BinaryView(taypeer_services::BinaryTarget),
    /// Read a validated inline icon from an already accessible local binary scope.
    IconPreview(taypeer_services::BinaryTarget),
    /// Stage or confirm a retryable binary command.
    EditBinary {
        /// Explicit source and target; binary bytes never enter JSON IPC.
        request: taypeer_services::BinaryRequest,
        /// Retry identity.
        operation: OperationId,
    },
    /// Import an explicitly selected stream into this exact active local form.
    /// The opaque capability is usable only by a platform worker, never a path.
    ImportSelectedAttachment {
        /// Exact local editor that owns the selected input.
        draft: taypeer_core::DraftId,
        /// Platform-owned input capability.
        input: u64,
        /// Declared complete plaintext length, checked while streaming.
        length: u64,
        /// Exact selected filename for a new attachment.
        name: String,
        /// Existing attachment identity when replacing only its content.
        replacement: Option<taypeer_core::AttachmentId>,
        /// Stable content-bound retry identity.
        operation: OperationId,
    },
    /// Export visible content through an explicitly selected platform output.
    ExportSelectedBinary {
        /// Visibility scope checked by the service.
        target: taypeer_services::BinaryTarget,
        /// Selected immutable content.
        blob: taypeer_core::BlobId,
        /// Platform-owned output capability.
        output: u64,
    },
    /// Explicitly export one accessible binary variant to a selected path.
    ExportBinary {
        /// Visibility scope checked by the service.
        target: taypeer_services::BinaryTarget,
        /// Selected content variant.
        blob: taypeer_core::BlobId,
        /// User-selected export destination.
        path: PathBuf,
        /// Explicit permission to replace a destination.
        overwrite: bool,
    },
    /// Quota, retained contents and separate local storage usage.
    StorageUsage,
    /// Rebuild retention and durably remove unreachable binary sections.
    CollectBlobs(OperationId),
    /// Explicit batch favicon acquisition, with a separate result per entry.
    GroupFavicons {
        /// Selected group.
        group: GroupId,
        /// Include descendant groups.
        recursive: bool,
        /// Explicitly replace existing icons.
        replace: bool,
        /// Stable batch retry identity.
        operation: OperationId,
    },
    /// Read the complete retained tree, conflicts and causal heads.
    Tree,
    /// List objects retained in the trash.
    Trash,
    /// Immediately retain an object in trash, reusing the exact selection on retry.
    TrashObject {
        /// Root object selected by the user.
        target: ObjectId,
        /// Stable identity for a lost response.
        operation: OperationId,
    },
    /// Inspect a retained object or immutable late source, with secrets masked.
    Inspect(InspectionTarget),
    /// Explicitly reveal exactly one inspected field alternative.
    RevealInspected {
        /// Scope of the reviewed data.
        target: InspectionTarget,
        /// Field address.
        field: taypeer_core::EntryField,
        /// Exact immutable variant origins.
        origins: Vec<String>,
    },
    /// Prepare an exact lifecycle selection.
    PrepareLifecycle {
        /// Lifecycle transition.
        action: LifecycleAction,
        /// Root object.
        target: ObjectId,
        /// Explicit restore destination.
        destination: Option<GroupId>,
    },
    /// Confirm precisely the prepared selection.
    ConfirmLifecycle {
        /// Reviewed causal context and exact generations.
        prepared: PreparedLifecycle,
        /// Durable idempotency key.
        operation: OperationId,
    },
    /// Move or resolve a group's placement.
    MoveGroup {
        /// Atomic placement request.
        request: GroupMove,
        /// Durable idempotency key.
        operation: OperationId,
    },
    /// Move or resolve an entry's destination.
    MoveEntry {
        /// Entry identity.
        entry: EntryId,
        /// Current destination.
        group: Option<GroupId>,
        /// Optional conflict review context.
        review: Option<Vec<String>>,
        /// Durable idempotency key.
        operation: OperationId,
    },
    /// Clone an active subtree.
    CloneGroup {
        /// Source group.
        group: GroupId,
        /// Destination parent.
        parent: Option<GroupId>,
        /// Optional new root name.
        name: Option<String>,
        /// Durable idempotency key.
        operation: OperationId,
    },
    /// List late sources awaiting explicit processing.
    PendingSources,
    /// Extract one late source into a fresh lifetime.
    RecoverSource {
        /// Explicit source, identity policy, destination and optional conflict choice.
        request: Box<RecoveryRequest>,
        /// Durable idempotency key.
        operation: OperationId,
    },
    /// Choose a reviewed generation after concurrent recovery.
    ResolveGeneration {
        /// Selected retained generation.
        address: ObjectAddress,
        /// Original review context.
        heads: Vec<String>,
        /// Durable idempotency key.
        operation: OperationId,
    },
    /// Read the database's groups.
    Groups,
    /// Create a group at the end of its parent's children.
    CreateGroup {
        /// Stable identity for retries of this exact request.
        operation: OperationId,
        /// Exact name.
        name: String,
        /// Optional parent.
        parent: Option<GroupId>,
    },
    /// Rename an existing group.
    RenameGroup {
        /// Stable identity for retries of this exact request.
        operation: OperationId,
        /// Target.
        id: GroupId,
        /// Exact name.
        name: String,
    },
    /// List a group or search the database.
    Entries {
        /// Optional group.
        group: Option<GroupId>,
        /// Search text, excluding secrets.
        query: String,
    },
    /// Read masked current details.
    Entry(EntryId),
    /// Create and confirm an entry with an addressed initial form.
    CreateEntry {
        /// Stable identity for retries of this exact request.
        operation: OperationId,
        /// Destination.
        group: Option<GroupId>,
        /// Initial fields.
        patch: EntryPatch,
    },
    /// Edit and confirm addressed fields, preserving omitted values.
    UpdateEntry {
        /// Stable identity for retries of this exact request.
        operation: OperationId,
        /// Target.
        id: EntryId,
        /// Changed fields.
        patch: EntryPatch,
    },
    /// Begin a new unconfirmed entry.
    BeginCreate(GroupId),
    /// Begin editing an existing entry, without returning its secret fields.
    BeginEdit(EntryId),
    /// Change only supplied draft fields.
    PatchDraft(EntryPatch),
    /// Read whether a draft is active or awaiting restoration, without its values.
    DraftStatus,
    /// Confirm the active draft.
    SaveDraft {
        /// Stable identity of this draft confirmation.
        operation: OperationId,
    },
    /// Discard an active or interrupted draft.
    DiscardDraft,
    /// Explicitly resume an interrupted draft.
    RestoreDraft,
    /// List masked saved revisions.
    History(EntryId),
    /// Read a masked historical revision.
    Revision {
        /// Entry.
        entry: EntryId,
        /// Saved revision.
        revision: RevisionId,
    },
    /// Clone the current entry with new identities.
    CloneEntry {
        /// Source entry.
        entry: EntryId,
        /// Destination group.
        group: Option<GroupId>,
        /// Optional replacement title.
        title: Option<String>,
        /// Idempotency key.
        operation: OperationId,
    },
    /// Restore an accessible revision as a new saved state.
    RestoreRevision {
        /// Entry.
        entry: EntryId,
        /// Source revision.
        revision: RevisionId,
        /// Explicit destination.
        group: Option<GroupId>,
        /// Idempotency key.
        operation: OperationId,
    },
    /// Permanently remove precisely selected history rows.
    PurgeHistory {
        /// Entry.
        entry: EntryId,
        /// Exact selected versions.
        revisions: std::collections::BTreeSet<RevisionId>,
        /// Idempotency key.
        operation: OperationId,
    },
    /// Read masked current conflicts and their causal review context.
    Conflicts(EntryId),
    /// Resolve only alternatives covered by the review context.
    ResolveConflicts {
        /// Reviewed context.
        context: ConflictContext,
        /// Chosen whole field values.
        fields: Vec<Resolution>,
        /// Idempotency key.
        operation: OperationId,
    },
    /// Explicitly reveal one currently retained conflict alternative.
    RevealConflict {
        /// Entry.
        entry: EntryId,
        /// Exact addressed field.
        field: taypeer_core::EntryField,
        /// Complete source identities returned by the masked review.
        origins: Vec<String>,
    },
    /// Explicitly reveal a current password.
    RevealPassword(EntryId),
    /// Explicitly reveal one attribute.
    RevealAttribute {
        /// Entry.
        entry: EntryId,
        /// Attribute.
        attribute: AttributeId,
    },
    /// Persist the draft and terminate the process, even on persistence failure.
    Lock,
}

impl std::fmt::Debug for Command {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("Command([REDACTED])")
    }
}

impl Command {
    /// Erase owned form input after dispatch; retained user copies remain caller-owned.
    pub fn erase_input(&mut self) {
        match self {
            Self::SetDatabaseInfo {
                name, description, ..
            } => {
                name.zeroize();
                description.zeroize();
            }
            Self::SaveGroup { form, .. } => {
                form.name.zeroize();
                form.description.zeroize();
            }
            Self::PatchGroupDraft { patch, .. } => {
                erase_text_patch(&mut patch.name);
                erase_text_patch(&mut patch.description);
            }
            Self::PatchDatabaseDraft { patch, .. } => {
                erase_text_patch(&mut patch.name);
                erase_text_patch(&mut patch.description);
            }
            Self::DraftExpiry(value) => value.zeroize(),
            Self::ImportSelectedAttachment { name, .. } => name.zeroize(),
            Self::PatchAttribute { patch, .. } => {
                patch.name.zeroize();
                if let taypeer_services::FieldUpdate::Set(value) = &mut patch.value {
                    value.zeroize();
                }
            }
            Self::CreateEntry { patch, .. }
            | Self::UpdateEntry { patch, .. }
            | Self::PatchDraft(patch) => patch.erase(),
            Self::ResolveConflicts { fields, .. } => {
                for resolution in fields {
                    match &mut resolution.value {
                        taypeer_core::FieldValue::Text(Some(text)) => text.zeroize(),
                        taypeer_core::FieldValue::Attribute(value) => value.value.zeroize(),
                        taypeer_core::FieldValue::Tags(tags) => {
                            for mut tag in std::mem::take(tags) {
                                tag.zeroize();
                            }
                        }
                        _ => {}
                    }
                }
            }
            Self::RecoverSource { request, .. } => {
                if let Some(fields) = &mut request.fields {
                    fields.title.zeroize();
                    fields.username.zeroize();
                    fields.password.zeroize();
                    fields.url.zeroize();
                    fields.notes.zeroize();
                    for mut tag in std::mem::take(&mut fields.tags) {
                        tag.zeroize();
                    }
                    for attribute in fields.attributes.values_mut() {
                        attribute.name.zeroize();
                        attribute.value.value.zeroize();
                    }
                }
            }
            _ => {}
        }
    }
}

fn erase_text_patch(patch: &mut taypeer_core::FieldUpdate<String>) {
    if let taypeer_core::FieldUpdate::Set(value) = patch {
        value.zeroize();
    }
}

/// Content-free failures suitable for a client to localize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize, thiserror::Error)]
#[error("{self:?}")]
pub enum RuntimeError {
    /// Native profile, secure credential storage or endpoint ownership failed.
    Profile(crate::profile::ProfileError),
    /// The shared application service rejected the command.
    Service(ServiceError),
    /// A private pipe or child process failed.
    Transport,
    /// Invalid private message structure.
    Protocol,
    /// An IPC message exceeded the bounded frame size.
    TooLarge,
    /// This worker has already been closed or invalidated.
    Closed,
    /// Access was revoked independently of command completion.
    SessionClosed(crate::session::LockReason),
    /// An in-flight command was interrupted; a previously started write may have committed.
    OperationInterrupted(crate::session::LockReason),
    /// Access is closed but operating-system process exit has not yet been confirmed.
    ShutdownUnconfirmed,
}
impl From<crate::profile::ProfileError> for RuntimeError {
    fn from(error: crate::profile::ProfileError) -> Self {
        if error == crate::profile::ProfileError::CommitUncertain {
            Self::Service(taypeer_services::ServiceError::Storage(
                taypeer_storage::Error::CommitUncertain,
            ))
        } else {
            Self::Profile(error)
        }
    }
}
impl From<ServiceError> for RuntimeError {
    fn from(value: ServiceError) -> Self {
        Self::Service(value)
    }
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Response {
    pub result: Result<serde_json::Value, RuntimeError>,
}

impl Response {
    pub(crate) fn into_result(mut self) -> Result<serde_json::Value, RuntimeError> {
        std::mem::replace(&mut self.result, Err(RuntimeError::Closed))
    }
}

impl Drop for Response {
    fn drop(&mut self) {
        if let Ok(value) = &mut self.result {
            erase_view(value);
        }
    }
}

/// Erase strings in a consumed view, including explicitly revealed values.
/// This clears this allocation only; external copies and terminal output remain caller-owned.
pub fn erase_view(value: &mut serde_json::Value) {
    match value {
        serde_json::Value::String(text) => text.zeroize(),
        serde_json::Value::Array(values) => values.iter_mut().for_each(erase_view),
        serde_json::Value::Object(values) => values.values_mut().for_each(erase_view),
        _ => {}
    }
}

pub(crate) fn write_frame(
    writer: &mut impl Write,
    message: &impl Serialize,
) -> Result<(), RuntimeError> {
    let bytes = Zeroizing::new(serde_json::to_vec(message).map_err(|_| RuntimeError::Protocol)?);
    write_payload(writer, &bytes)
}

pub(crate) fn write_payload(writer: &mut impl Write, bytes: &[u8]) -> Result<(), RuntimeError> {
    if bytes.len() > MAX_MESSAGE {
        return Err(RuntimeError::TooLarge);
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|()| writer.write_all(bytes))
        .and_then(|()| writer.flush())
        .map_err(|_| RuntimeError::Transport)
}

pub(crate) fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T, RuntimeError> {
    let bytes = read_raw_frame(reader)?;
    serde_json::from_slice(&bytes[4..]).map_err(|_| RuntimeError::Protocol)
}

pub(crate) fn read_raw_frame(reader: &mut impl Read) -> Result<Zeroizing<Vec<u8>>, RuntimeError> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| RuntimeError::Transport)?;
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_MESSAGE {
        return Err(RuntimeError::TooLarge);
    }
    let mut bytes = Zeroizing::new(vec![0; length + 4]);
    bytes[..4].copy_from_slice(&header);
    reader
        .read_exact(&mut bytes[4..])
        .map_err(|_| RuntimeError::Transport)?;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn untrusted_lengths_and_partial_frames_are_rejected() {
        assert!(matches!(
            read_frame::<Command>(&mut (u32::MAX.to_le_bytes().as_slice())),
            Err(RuntimeError::TooLarge)
        ));
        assert!(matches!(
            read_frame::<Command>(&mut [8, 0, 0, 0, 1].as_slice()),
            Err(RuntimeError::Transport)
        ));
    }
}
