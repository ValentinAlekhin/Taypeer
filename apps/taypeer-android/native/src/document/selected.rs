//! Capabilities for explicitly selected plaintext streams, separate from ciphertext persistence.
use super::{DocumentSession, EntryEditor, failure, session::identifier};
use crate::AndroidError;
use std::{
    collections::BTreeMap,
    io::{Read, Write},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, AtomicU64, Ordering},
    },
};
use taypeer_core::{AttachmentId, BlobId, DraftId, EntryId, OperationId, RevisionId};
use taypeer_runtime::{Command, WorkerControl};
use taypeer_services::{AttachmentEdit, BinaryEdit, BinaryRequest, BinaryTarget};

/// One explicit SAF selection. Its contents are never copied to a host temporary file.
#[uniffi::export(callback_interface)]
pub trait SelectedInput: Send + Sync {
    /// Exact declared size, checked against the shared attachment policy before reading.
    fn length(&self) -> Result<u64, AndroidError>;
    /// Read at most 64 KiB at an independent offset.
    fn read(&self, offset: u64, count: u32) -> Result<Vec<u8>, AndroidError>;
    /// Release this selection; implementations must tolerate repeated close.
    fn close(&self);
}
/// One explicitly selected export destination, with no document or host path authority.
#[uniffi::export(callback_interface)]
pub trait SelectedOutput: Send + Sync {
    /// Consume at most 64 KiB at the specified offset.
    fn write(&self, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError>;
    /// Flush and confirm durability before reporting export success.
    fn finish(&self) -> Result<(), AndroidError>;
    /// Release this selection after success or failure.
    fn close(&self);
}
/// Separate private Binder port for explicitly selected binary operations.
#[uniffi::export(callback_interface)]
pub trait SelectedTransfersRemote: Send + Sync {
    /// Read one active input capability, bounded by 64 KiB.
    fn read(&self, id: u64, offset: u64, count: u32) -> Result<Vec<u8>, AndroidError>;
    /// Write one active output capability, bounded by 64 KiB.
    fn write(&self, id: u64, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError>;
    /// Confirm the selected output; a lost reply must not become a successful export.
    fn finish(&self, id: u64) -> Result<(), AndroidError>;
}
enum Selection {
    Input {
        stream: Arc<dyn SelectedInput>,
        length: u64,
    },
    Output(Arc<dyn SelectedOutput>),
}
impl Drop for Selection {
    fn drop(&mut self) {
        match self {
            Self::Input { stream, .. } => stream.close(),
            Self::Output(stream) => stream.close(),
        }
    }
}
/// Host-owned ephemeral capabilities for exactly one supervised worker generation.
#[derive(uniffi::Object, Default)]
pub struct SelectedTransfersHost {
    next: AtomicU64,
    revoked: AtomicBool,
    control: Mutex<Option<WorkerControl>>,
    entries: Mutex<BTreeMap<u64, Arc<Selection>>>,
}
impl SelectedTransfersHost {
    pub(super) fn bind(&self, control: WorkerControl) -> Result<(), AndroidError> {
        *self.control.lock().map_err(failure)? = Some(control);
        Ok(())
    }
    pub(super) fn revoke(&self) {
        self.revoked.store(true, Ordering::Release);
    }
    fn checked(&self) -> Result<(), AndroidError> {
        if self.revoked.load(Ordering::Acquire)
            || !self
                .control
                .lock()
                .map_err(failure)?
                .as_ref()
                .is_some_and(WorkerControl::is_open)
        {
            return Err(AndroidError::Runtime);
        }
        Ok(())
    }
    fn insert(self: &Arc<Self>, selection: Selection) -> Result<SelectionLease, AndroidError> {
        self.checked()?;
        let id = self
            .next
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |value| {
                value.checked_add(1)
            })
            .map_err(failure)?
            + 1;
        self.entries
            .lock()
            .map_err(failure)?
            .insert(id, Arc::new(selection));
        Ok(SelectionLease {
            host: Arc::clone(self),
            id,
        })
    }
    fn get(&self, id: u64) -> Result<Arc<Selection>, AndroidError> {
        self.checked()?;
        self.entries
            .lock()
            .map_err(failure)?
            .get(&id)
            .cloned()
            .ok_or(AndroidError::InvalidOptions)
    }
}
struct SelectionLease {
    host: Arc<SelectedTransfersHost>,
    id: u64,
}
impl Drop for SelectionLease {
    fn drop(&mut self) {
        let removed = self
            .host
            .entries
            .lock()
            .ok()
            .and_then(|mut entries| entries.remove(&self.id));
        // Closing an OS descriptor or callback must never run with the registry lock held.
        drop(removed);
    }
}
#[uniffi::export]
impl SelectedTransfersHost {
    /// Read only a currently registered input; IDs never escape their worker generation.
    pub fn read(&self, id: u64, offset: u64, count: u32) -> Result<Vec<u8>, AndroidError> {
        if count as usize > crate::descriptors::TRANSFER_CHUNK {
            return Err(AndroidError::InvalidOptions);
        }
        let selection = self.get(id)?;
        let Selection::Input { stream, length } = &*selection else {
            return Err(AndroidError::InvalidOptions);
        };
        let limit = length
            .checked_sub(offset)
            .ok_or(AndroidError::InvalidOptions)?
            // BlobStore probes one byte after the declared size to reject a
            // growing provider. Clamping that probe to zero would hide extra data.
            .saturating_add(1)
            .min(count.into()) as usize;
        let mut bytes = zeroize::Zeroizing::new(stream.read(offset, limit as u32)?);
        if bytes.len() > limit {
            return Err(AndroidError::InvalidFile);
        }
        self.checked()?;
        Ok(std::mem::take(&mut *bytes))
    }
    /// Write only a currently registered destination; no capability grants access to storage files.
    pub fn write(&self, id: u64, offset: u64, bytes: Vec<u8>) -> Result<u32, AndroidError> {
        if bytes.len() > crate::descriptors::TRANSFER_CHUNK {
            return Err(AndroidError::InvalidOptions);
        }
        let selection = self.get(id)?;
        let Selection::Output(stream) = &*selection else {
            return Err(AndroidError::InvalidOptions);
        };
        let count = bytes.len();
        let written = stream.write(offset, bytes)?;
        if written as usize > count {
            return Err(AndroidError::InvalidFile);
        }
        self.checked()?;
        Ok(written)
    }
    /// Success requires the selected provider's explicit durability confirmation.
    pub fn finish(&self, id: u64) -> Result<(), AndroidError> {
        let selection = self.get(id)?;
        let Selection::Output(stream) = &*selection else {
            return Err(AndroidError::InvalidOptions);
        };
        stream.finish()?;
        self.checked()
    }
}

/// An immutable content variant, whose identity alone does not grant export authority.
#[derive(uniffi::Record)]
pub struct AttachmentContent {
    /// Opaque content identity.
    pub blob: String,
    /// Verified locally available size; absent while awaiting encrypted contents.
    pub bytes: Option<u64>,
}
/// Selected attachment metadata. Historical alternatives are read through an explicit revision.
#[derive(uniffi::Record)]
pub struct AttachmentRow {
    /// Stable identity independent of the displayed name.
    pub attachment: String,
    /// Selected exact filename.
    pub name: String,
    /// Accessible contents for this explicit visibility scope.
    pub contents: Vec<AttachmentContent>,
}
#[uniffi::export]
impl DocumentSession {
    /// Stage an explicitly selected stream into the active form using the shared service.
    pub fn import_attachment(
        &self,
        draft: String,
        name: String,
        replacement: Option<String>,
        operation: String,
        input: Box<dyn SelectedInput>,
    ) -> Result<EntryEditor, AndroidError> {
        let input: Arc<dyn SelectedInput> = Arc::from(input);
        let length = input.length();
        let selection = Selection::Input {
            stream: input,
            length: length.as_ref().copied().unwrap_or(0),
        };
        let length = length?;
        let lease = self.selected.insert(selection)?;
        if name.is_empty() || name.len() > 65536 {
            return Err(AndroidError::InvalidOptions);
        }
        self.command::<serde_json::Value>(Command::ImportSelectedAttachment {
            draft: DraftId::new(identifier(draft)?),
            input: lease.id,
            length,
            name,
            replacement: replacement
                .map(identifier)
                .transpose()?
                .map(AttachmentId::new),
            operation: OperationId::new(identifier(operation)?),
        })?;
        self.editor()
    }
    /// Rename one identity without re-reading its contents.
    pub fn rename_attachment(
        &self,
        draft: String,
        attachment: String,
        name: String,
        operation: String,
    ) -> Result<EntryEditor, AndroidError> {
        if name.is_empty() || name.len() > 65536 {
            return Err(AndroidError::InvalidOptions);
        }
        self.edit_attachment(
            draft,
            AttachmentEdit::Rename {
                attachment: AttachmentId::new(identifier(attachment)?),
                name,
            },
            operation,
        )
    }
    /// Remove only the selected identity; accessible history retains its binary contents.
    pub fn remove_attachment(
        &self,
        draft: String,
        attachment: String,
        operation: String,
    ) -> Result<EntryEditor, AndroidError> {
        self.edit_attachment(
            draft,
            AttachmentEdit::Remove {
                attachment: AttachmentId::new(identifier(attachment)?),
            },
            operation,
        )
    }
    /// Read selected names and content availability without exporting any bytes.
    pub fn form_attachments(&self) -> Result<Vec<AttachmentRow>, AndroidError> {
        self.attachments(BinaryTarget::Draft)
    }
    /// Read selected saved or historical attachment metadata.
    pub fn entry_attachments(
        &self,
        entry: String,
        revision: Option<String>,
    ) -> Result<Vec<AttachmentRow>, AndroidError> {
        self.attachments(attachment_target(entry, revision)?)
    }
    /// Export an explicitly visible binary content to an explicitly selected destination.
    pub fn export_attachment(
        &self,
        entry: String,
        revision: Option<String>,
        blob: String,
        output: Box<dyn SelectedOutput>,
    ) -> Result<(), AndroidError> {
        let selection = Selection::Output(Arc::from(output));
        let lease = self.selected.insert(selection)?;
        self.command::<serde_json::Value>(Command::ExportSelectedBinary {
            target: attachment_target(entry, revision)?,
            blob: BlobId::new(identifier(blob)?),
            output: lease.id,
        })?;
        Ok(())
    }
}
impl DocumentSession {
    fn edit_attachment(
        &self,
        draft: String,
        edit: AttachmentEdit,
        operation: String,
    ) -> Result<EntryEditor, AndroidError> {
        let current = self.editor()?;
        if current.form.draft != identifier(draft)? {
            return Err(AndroidError::InvalidOptions);
        }
        self.command::<serde_json::Value>(Command::EditBinary {
            request: BinaryRequest {
                target: BinaryTarget::Draft,
                edit: BinaryEdit::Attachment(edit),
                review: None,
            },
            operation: OperationId::new(identifier(operation)?),
        })?;
        self.editor()
    }
    fn attachments(&self, target: BinaryTarget) -> Result<Vec<AttachmentRow>, AndroidError> {
        let view: taypeer_services::BinaryView =
            self.command(Command::BinaryView(target.clone()))?;
        // Current entry fields are the deterministic projection; original alternatives remain historical.
        let selected = match target {
            BinaryTarget::Entry(id) => Some(
                self.command::<taypeer_services::EntryView>(Command::Entry(id))?
                    .attachments,
            ),
            BinaryTarget::Revision { entry, revision } => Some(
                self.command::<taypeer_services::EntryView>(Command::Revision { entry, revision })?
                    .attachments,
            ),
            _ => None,
        };
        Ok(view
            .attachments
            .into_iter()
            .filter_map(|row| {
                let current = selected
                    .as_ref()
                    .and_then(|all| all.iter().find(|a| a.id == row.id));
                if selected.is_some() && current.is_none() {
                    return None;
                }
                let name = current
                    .map(|a| a.name.clone())
                    .or_else(|| row.names.into_iter().next())?;
                Some(AttachmentRow {
                    attachment: row.id.to_string(),
                    name,
                    contents: row
                        .contents
                        .into_iter()
                        .filter(|c| current.is_none_or(|a| a.blob == c.id))
                        .map(|c| AttachmentContent {
                            blob: c.id.to_string(),
                            bytes: c.bytes,
                        })
                        .collect(),
                })
            })
            .collect())
    }
}
fn attachment_target(
    entry: String,
    revision: Option<String>,
) -> Result<BinaryTarget, AndroidError> {
    let entry = EntryId::new(identifier(entry)?);
    Ok(match revision {
        Some(id) => BinaryTarget::Revision {
            entry,
            revision: RevisionId::new(identifier(id)?),
        },
        None => BinaryTarget::Entry(entry),
    })
}

pub(super) struct RemoteInput {
    pub(super) port: Arc<dyn SelectedTransfersRemote>,
    pub(super) id: u64,
    pub(super) offset: u64,
}
impl Read for RemoteInput {
    fn read(&mut self, destination: &mut [u8]) -> std::io::Result<usize> {
        if destination.is_empty() {
            return Ok(0);
        }
        let count = destination.len().min(crate::descriptors::TRANSFER_CHUNK);
        let bytes = zeroize::Zeroizing::new(
            self.port
                .read(self.id, self.offset, count as u32)
                .map_err(|_| std::io::ErrorKind::Other)?,
        );
        if bytes.len() > count {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        destination[..bytes.len()].copy_from_slice(&bytes);
        self.offset = self
            .offset
            .checked_add(bytes.len() as u64)
            .ok_or(std::io::ErrorKind::InvalidData)?;
        Ok(bytes.len())
    }
}
pub(super) struct RemoteOutput {
    pub(super) port: Arc<dyn SelectedTransfersRemote>,
    pub(super) id: u64,
    pub(super) offset: u64,
}
impl Write for RemoteOutput {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(crate::descriptors::TRANSFER_CHUNK);
        let written = self
            .port
            .write(self.id, self.offset, bytes[..count].to_vec())
            .map_err(|_| std::io::ErrorKind::Other)? as usize;
        if written > count {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        self.offset = self
            .offset
            .checked_add(written as u64)
            .ok_or(std::io::ErrorKind::InvalidData)?;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl taypeer_runtime::platform_worker::SelectedOutput for RemoteOutput {
    fn finish(&mut self) -> Result<(), taypeer_runtime::RuntimeError> {
        self.port.finish(self.id).map_err(|error| {
            let error = match error {
                AndroidError::CommitUncertain => taypeer_storage::Error::CommitUncertain,
                _ => taypeer_storage::Error::Io,
            };
            taypeer_runtime::RuntimeError::Service(taypeer_services::ServiceError::Storage(error))
        })
    }
}

#[cfg(test)]
mod tests;
