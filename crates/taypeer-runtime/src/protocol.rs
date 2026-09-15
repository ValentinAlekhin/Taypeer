//! Private, length-prefixed JSON messages. This is not the network/file protocol.

use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{
    io::{Read, Write},
    path::PathBuf,
};
use taypeer_core::{AttributeId, EntryId, GroupId, OperationId, RevisionId};
use taypeer_services::{
    ConflictContext, EntryPatch, GroupMove, InspectionTarget, LifecycleAction, ObjectAddress,
    ObjectId, PreparedLifecycle, RecoveryRequest, Resolution, ServiceError,
};
use zeroize::{Zeroize, Zeroizing};

const MAX_MESSAGE: usize = 16 * 1024 * 1024;

#[derive(Serialize, Deserialize)]
pub(crate) struct Boot {
    pub path: PathBuf,
    pub password: String,
    pub create_name: Option<String>,
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
        group: GroupId,
        /// Durable exact-intent retry identity.
        operation: OperationId,
    },
    /// Read public signed device/control state, without acquiring another credential.
    Authority,
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
    /// Stage or confirm a retryable binary command.
    EditBinary {
        /// Explicit source and target; binary bytes never enter JSON IPC.
        request: taypeer_services::BinaryRequest,
        /// Retry identity.
        operation: OperationId,
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
        group: GroupId,
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
        /// Exact name.
        name: String,
        /// Optional parent.
        parent: Option<GroupId>,
    },
    /// Rename an existing group.
    RenameGroup {
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
        /// Destination.
        group: GroupId,
        /// Initial fields.
        patch: EntryPatch,
    },
    /// Edit and confirm addressed fields, preserving omitted values.
    UpdateEntry {
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
    SaveDraft,
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
        group: GroupId,
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
        group: GroupId,
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

/// Content-free failures suitable for a client to localize.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
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
}
impl From<crate::profile::ProfileError> for RuntimeError {
    fn from(error: crate::profile::ProfileError) -> Self {
        Self::Profile(error)
    }
}
impl From<ServiceError> for RuntimeError {
    fn from(value: ServiceError) -> Self {
        Self::Service(value)
    }
}
impl std::fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for RuntimeError {}

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
    if bytes.len() > MAX_MESSAGE {
        return Err(RuntimeError::TooLarge);
    }
    writer
        .write_all(&(bytes.len() as u32).to_le_bytes())
        .and_then(|()| writer.write_all(&bytes))
        .and_then(|()| writer.flush())
        .map_err(|_| RuntimeError::Transport)
}

pub(crate) fn read_frame<T: DeserializeOwned>(reader: &mut impl Read) -> Result<T, RuntimeError> {
    let mut header = [0; 4];
    reader
        .read_exact(&mut header)
        .map_err(|_| RuntimeError::Transport)?;
    let length = u32::from_le_bytes(header) as usize;
    if length > MAX_MESSAGE {
        return Err(RuntimeError::TooLarge);
    }
    let mut bytes = Zeroizing::new(vec![0; length]);
    reader
        .read_exact(&mut bytes)
        .map_err(|_| RuntimeError::Transport)?;
    serde_json::from_slice(&bytes).map_err(|_| RuntimeError::Protocol)
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
