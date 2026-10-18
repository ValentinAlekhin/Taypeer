use super::{decode, digest, encode, failure, runtime};
use crate::{AndroidError, CiphertextFiles, descriptors};
use std::{path::Path, sync::Arc};
use taypeer_storage::{
    ArchiveSeed, ArchiveSnapshot, CipherPersistence, EncryptedObject, PreparedCommit,
    TemporaryStorage,
};
use taypeer_trust::{ControlChain, Digest, SignedControl};

mod recovery;
pub use recovery::{CipherRecoveryArchive, CipherRecoverySource};

/// Immutable descriptor lease plus verified public generation identity.
#[derive(uniffi::Record)]
pub struct CipherSnapshot {
    /// Lease in the recipient's ciphertext table.
    pub file: u64,
    /// Encoded generation digest.
    pub fingerprint: String,
    /// Pinned signed trust lineage.
    pub root: String,
    /// Local-only draft binding.
    pub working_copy: String,
}
/// Encrypted genesis; metadata contains signed controls, never a decrypted document.
#[derive(uniffi::Record)]
pub struct CipherSeed {
    /// Bounded serialization of signed public controls.
    pub controls: Vec<u8>,
    /// Leases for independently signed encrypted objects.
    pub objects: Vec<u64>,
    /// Initial encrypted checkpoint identity.
    pub checkpoint: String,
    /// Manager-authenticated encrypted baseline identity.
    pub baseline: String,
}
/// Typed encrypted commit. The host verifies every object and rejects stale generations.
#[derive(uniffi::Record)]
pub struct CipherCommit {
    /// Exact generation captured by the worker.
    pub expected: String,
    /// Exact signed authority head.
    pub control: String,
    /// Signed public controls, bounded independently from object bytes.
    pub controls: Vec<u8>,
    /// Ciphertext leases consumed by the host.
    pub objects: Vec<u64>,
    /// Explicit encrypted objects eligible for release.
    pub remove: Vec<String>,
    /// New checkpoint identity.
    pub checkpoint: String,
    /// Active manager baseline identity.
    pub baseline: String,
    /// Bounded portable ciphertext operation journal.
    pub journal: Vec<u8>,
}

/// Binder adapter available only to the isolated worker. The UI receives no keys.
#[uniffi::export(callback_interface)]
pub trait DocumentPersistence: Send + Sync {
    /// Transfer a fresh immutable generation into this worker's descriptor table.
    fn snapshot(&self) -> Result<CipherSnapshot, AndroidError>;
    /// Publish a complete encrypted initial generation.
    fn create(&self, seed: CipherSeed) -> Result<(), AndroidError>;
    /// Durably commit all ciphertext objects and return the verified new generation.
    fn commit(&self, commit: CipherCommit) -> Result<CipherSnapshot, AndroidError>;
    /// Read bounded epoch-encrypted credentials; missing differs from corrupt or denied.
    fn credential(&self, scope: Vec<u8>, prepared: bool) -> Result<Option<Vec<u8>>, AndroidError>;
    /// Publish worker-encrypted credential bytes with a durable acknowledgement.
    fn publish_credential(
        &self,
        scope: Vec<u8>,
        cipher: Vec<u8>,
        prepared: bool,
    ) -> Result<(), AndroidError>;
    /// Complete the prepared credential after the database epoch is durable.
    fn finalize_credential(
        &self,
        scope: Vec<u8>,
        promote_prepared: bool,
    ) -> Result<(), AndroidError>;
    /// Private ciphertext-only transport authority; never an author or read key.
    fn activate_credentials(
        &self,
        capability: Vec<u8>,
        authenticated: Option<String>,
    ) -> Result<(), AndroidError>;
    /// Activate an exact retained recovery source through the independently unlocked target.
    fn activate_recovery_credentials(
        &self,
        source: Vec<u8>,
        capability: Vec<u8>,
    ) -> Result<(), AndroidError>;
    /// Durably retain the encrypted local collection.
    fn save_draft(&self, file: u64) -> Result<(), AndroidError>;
    /// Transfer an optional encrypted collection into the worker's table.
    fn load_draft(&self) -> Result<Option<u64>, AndroidError>;
    /// Durably remove the local collection.
    fn discard_draft(&self) -> Result<(), AndroidError>;
    /// Freeze one authenticated source. The returned archive is ciphertext-only.
    fn freeze_recovery(
        &self,
        _operation: String,
        _authenticated: String,
    ) -> Result<CipherRecoverySource, AndroidError> {
        Err(AndroidError::Runtime)
    }
    /// Resume the prior immutable encrypted source before password authentication.
    fn recovery_source(
        &self,
        _operation: String,
    ) -> Result<Option<CipherRecoverySource>, AndroidError> {
        Err(AndroidError::Runtime)
    }
    /// Read the actual candidate; a missing candidate differs from a failed read.
    fn recovery_candidate(
        &self,
        _intent: Vec<u8>,
    ) -> Result<Option<CipherRecoveryArchive>, AndroidError> {
        Err(AndroidError::Runtime)
    }
    /// Publish verified encrypted objects without activating catalog or network.
    fn publish_recovery(
        &self,
        _intent: Vec<u8>,
        _seed: CipherSeed,
    ) -> Result<Vec<u8>, AndroidError> {
        Err(AndroidError::Runtime)
    }
    /// Publish a new trust set plus independently wrapped local credentials.
    fn publish_recovery_credentials(
        &self,
        _intent: Vec<u8>,
        _seed: CipherSeed,
        _cipher: Vec<u8>,
        _capability: Vec<u8>,
    ) -> Result<Vec<u8>, AndroidError> {
        Err(AndroidError::Runtime)
    }
    /// Confirm real ciphertext durability and protected receipt, never a cached locator.
    fn confirm_recovery(&self, _publication: Vec<u8>) -> Result<(), AndroidError> {
        Err(AndroidError::Runtime)
    }
}

/// Host-only descriptor adapter. This object never opens a plaintext DatabaseService.
#[derive(uniffi::Object)]
pub struct CipherWriter {
    pub(crate) writer: Arc<taypeer_runtime::platform_worker::PlatformCipherWriter>,
    pub(crate) files: Arc<dyn CiphertextFiles>,
}
impl CipherWriter {
    fn sources(&self, ids: Vec<u64>) -> Result<Vec<taypeer_storage::CiphertextFile>, AndroidError> {
        let sources: Vec<_> = ids
            .into_iter()
            .map(|id| descriptors::source(Arc::clone(&self.files), id))
            .collect();
        if sources.len() > 4096 {
            return Err(AndroidError::InvalidFile);
        }
        Ok(sources)
    }
    fn objects(
        &self,
        sources: Vec<taypeer_storage::CiphertextFile>,
        chain: &ControlChain,
    ) -> Result<Vec<EncryptedObject>, AndroidError> {
        sources
            .into_iter()
            .map(|source| {
                EncryptedObject::open_source(
                    source,
                    chain,
                    descriptors::temporary(Arc::clone(&self.files)),
                )
                .map_err(failure)
            })
            .collect()
    }
    fn transfer(&self, reader: impl std::io::Read) -> Result<u64, AndroidError> {
        Transfer {
            files: Arc::clone(&self.files),
        }
        .copy(reader)
    }
    fn view(&self, snapshot: ArchiveSnapshot) -> Result<CipherSnapshot, AndroidError> {
        let root = snapshot.chain().root().map_err(failure)?.to_string();
        let working_copy = self.writer.working_copy().map_err(runtime)?.to_string();
        let file = self.files.allocate()?;
        // Ownership stays in the host table until Binder duplicates and releases it.
        let mut output = TableWrite {
            files: Arc::clone(&self.files),
            id: file,
            offset: 0,
        };
        if let Err(error) = snapshot.copy_ciphertext(&mut output) {
            self.files.release(file);
            return Err(failure(error));
        }
        Ok(CipherSnapshot {
            file,
            fingerprint: snapshot.fingerprint().to_string(),
            root,
            working_copy,
        })
    }
}

#[uniffi::export]
impl CipherWriter {
    /// Snapshot contains ciphertext and public identities only.
    pub fn snapshot(&self) -> Result<CipherSnapshot, AndroidError> {
        self.view(self.writer.snapshot().map_err(runtime)?)
    }
    /// Create using verified signed genesis and encrypted object leases.
    pub fn create(&self, seed: CipherSeed) -> Result<(), AndroidError> {
        let sources = self.sources(seed.objects)?;
        let controls: Vec<SignedControl> = decode(&seed.controls)?;
        let root = controls
            .first()
            .ok_or(AndroidError::InvalidFile)?
            .hash()
            .map_err(failure)?;
        let chain = ControlChain::validate(controls.clone(), root).map_err(failure)?;
        self.writer
            .create(ArchiveSeed {
                objects: self.objects(sources, &chain)?,
                controls,
                checkpoint: digest(&seed.checkpoint)?,
                baseline: digest(&seed.baseline)?,
            })
            .map_err(runtime)
    }
    /// Durability is confirmed by the shared coordinator before returning.
    pub fn commit(&self, commit: CipherCommit) -> Result<CipherSnapshot, AndroidError> {
        let sources = self.sources(commit.objects)?;
        let controls: Vec<SignedControl> = decode(&commit.controls)?;
        let root = self
            .writer
            .snapshot()
            .map_err(runtime)?
            .chain()
            .root()
            .map_err(failure)?;
        let chain = ControlChain::validate(controls.clone(), root).map_err(failure)?;
        if commit.remove.len() > 4096 {
            return Err(AndroidError::InvalidFile);
        }
        let request = PreparedCommit {
            expected: digest(&commit.expected)?,
            control: digest(&commit.control)?,
            controls,
            objects: self.objects(sources, &chain)?,
            remove: commit
                .remove
                .iter()
                .map(|value| digest(value))
                .collect::<Result<_, _>>()?,
            checkpoint: digest(&commit.checkpoint)?,
            baseline: digest(&commit.baseline)?,
            journal: decode(&commit.journal)?,
        };
        let snapshot = self.writer.commit(request).map_err(runtime)?;
        // Publication has succeeded. Failure to transfer its reply is uncertain
        // to the worker, which must reconcile the file and its operation receipt.
        self.view(snapshot)
            .map_err(|_| AndroidError::CommitUncertain)
    }
    /// Private process capability; never expose this object to Compose or saved state.
    pub fn credential(
        &self,
        scope: Vec<u8>,
        prepared: bool,
    ) -> Result<Option<Vec<u8>>, AndroidError> {
        let scope = decode(&scope)?;
        self.writer.credential(&scope, prepared).map_err(runtime)
    }
    /// Persist ciphertext produced only inside the isolated document process.
    pub fn publish_credential(
        &self,
        scope: Vec<u8>,
        cipher: Vec<u8>,
        prepared: bool,
    ) -> Result<(), AndroidError> {
        let scope = decode(&scope)?;
        if cipher.len() > 128 * 1024 {
            return Err(AndroidError::InvalidFile);
        }
        self.writer
            .publish_credential(&scope, &cipher, prepared)
            .map_err(runtime)
    }
    /// Confirm prepared publication before the worker can acknowledge rotation.
    pub fn finalize_credential(
        &self,
        scope: Vec<u8>,
        promote_prepared: bool,
    ) -> Result<(), AndroidError> {
        self.writer
            .finalize_credential(&decode(&scope)?, promote_prepared)
            .map_err(runtime)
    }
    /// Activate the independent transport role without receiving an author seed.
    pub fn activate_credentials(
        &self,
        capability: Vec<u8>,
        authenticated: Option<String>,
    ) -> Result<(), AndroidError> {
        let capability = zeroize::Zeroizing::new(capability);
        let capability = decode(&capability)?;
        self.writer
            .activate_credentials(
                capability,
                authenticated.as_deref().map(digest).transpose()?,
            )
            .map_err(runtime)
    }
    /// No source author or key is returned to Binder or the application host.
    pub fn activate_recovery_credentials(
        &self,
        source: Vec<u8>,
        capability: Vec<u8>,
    ) -> Result<(), AndroidError> {
        let source = decode(&source)?;
        let capability = zeroize::Zeroizing::new(capability);
        self.writer
            .activate_recovery_credentials(&source, decode(&capability)?)
            .map_err(runtime)
    }
    /// Save one encrypted local-only collection.
    pub fn save_draft(&self, file: u64) -> Result<(), AndroidError> {
        let source = descriptors::source(Arc::clone(&self.files), file);
        let snapshot = self.writer.snapshot().map_err(runtime)?;
        let object = EncryptedObject::open_source(
            source,
            snapshot.chain(),
            descriptors::temporary(Arc::clone(&self.files)),
        )
        .map_err(failure)?;
        self.writer.save_draft(&object).map_err(runtime)
    }
    /// Optional ciphertext lease; absent differs from a failed read.
    pub fn load_draft(&self) -> Result<Option<u64>, AndroidError> {
        self.writer
            .load_draft()
            .map_err(runtime)?
            .map(|object| self.transfer(object.reader().map_err(failure)?))
            .transpose()
    }
    /// Explicit local collection removal.
    pub fn discard_draft(&self) -> Result<(), AndroidError> {
        self.writer.discard_draft().map_err(runtime)
    }
}

pub(super) struct RemoteDocument {
    pub port: Arc<dyn DocumentPersistence>,
    pub files: Arc<dyn CiphertextFiles>,
    pub copy: std::sync::Mutex<Option<Digest>>,
    pub selected: Arc<dyn super::SelectedTransfersRemote>,
}
impl RemoteDocument {
    fn accept(&self, view: CipherSnapshot) -> Result<ArchiveSnapshot, AndroidError> {
        let source = descriptors::source(Arc::clone(&self.files), view.file);
        let copy = digest(&view.working_copy)?;
        if self
            .copy
            .lock()
            .map_err(failure)?
            .is_some_and(|prior| prior != copy)
        {
            return Err(AndroidError::InvalidFile);
        }
        let snapshot = ArchiveSnapshot::from_source(
            source,
            Some(digest(&view.root)?),
            descriptors::temporary(Arc::clone(&self.files)),
        )
        .map_err(failure)?;
        if snapshot.fingerprint() != digest(&view.fingerprint)? {
            return Err(AndroidError::InvalidFile);
        }
        let mut binding = self.copy.lock().map_err(failure)?;
        if binding.is_some_and(|prior| prior != copy) {
            return Err(AndroidError::InvalidFile);
        }
        *binding = Some(copy);
        Ok(snapshot)
    }
    fn spool(&self, objects: Vec<EncryptedObject>) -> Result<TransferBatch, AndroidError> {
        let writer = Transfer {
            files: Arc::clone(&self.files),
        };
        let mut batch = TransferBatch {
            files: Arc::clone(&self.files),
            ids: Vec::new(),
        };
        for object in objects {
            batch
                .ids
                .push(writer.copy(object.reader().map_err(failure)?)?);
        }
        Ok(batch)
    }
}
// Every outgoing lease remains owned until the synchronous Binder call returns,
// including failures while preparing metadata or opening a later object.
struct TransferBatch {
    files: Arc<dyn CiphertextFiles>,
    ids: Vec<u64>,
}
impl Drop for TransferBatch {
    fn drop(&mut self) {
        for id in &self.ids {
            self.files.release(*id);
        }
    }
}

struct Transfer {
    files: Arc<dyn CiphertextFiles>,
}
impl Transfer {
    fn copy(&self, mut reader: impl std::io::Read) -> Result<u64, AndroidError> {
        let id = self.files.allocate()?;
        let result = (|| {
            let mut offset = 0;
            let mut bytes = [0; descriptors::TRANSFER_CHUNK];
            loop {
                let count = reader.read(&mut bytes).map_err(failure)?;
                if count == 0 {
                    return Ok(id);
                }
                let mut done = 0;
                while done < count {
                    let written =
                        self.files.write(id, offset, bytes[done..count].to_vec())? as usize;
                    if written == 0 || written > count - done {
                        return Err(AndroidError::InvalidFile);
                    }
                    done += written;
                    offset += written as u64;
                }
            }
        })();
        if result.is_err() {
            self.files.release(id);
        }
        result
    }
}
fn storage(error: AndroidError) -> taypeer_storage::Error {
    match error {
        AndroidError::StorageChanged => taypeer_storage::Error::Changed,
        AndroidError::StorageBusy => taypeer_storage::Error::Busy,
        AndroidError::CommitUncertain => taypeer_storage::Error::CommitUncertain,
        AndroidError::AlreadyExists => taypeer_storage::Error::AlreadyExists,
        AndroidError::InvalidFile | AndroidError::InvalidOptions => {
            taypeer_storage::Error::InvalidFile
        }
        _ => taypeer_storage::Error::Io,
    }
}

struct TableWrite {
    files: Arc<dyn CiphertextFiles>,
    id: u64,
    offset: u64,
}
impl std::io::Write for TableWrite {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        let count = bytes.len().min(descriptors::TRANSFER_CHUNK);
        let written = self
            .files
            .write(self.id, self.offset, bytes[..count].to_vec())
            .map_err(|_| std::io::ErrorKind::Other)? as usize;
        if written > count {
            return Err(std::io::ErrorKind::InvalidData.into());
        }
        self.offset = self
            .offset
            .checked_add(written as u64)
            .ok_or(std::io::ErrorKind::InvalidInput)?;
        Ok(written)
    }
    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}
impl CipherPersistence for RemoteDocument {
    fn snapshot(&self) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.accept(self.port.snapshot().map_err(storage)?)
            .map_err(storage)
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        let ids = self.spool(request.objects).map_err(storage)?;
        let view = self.port.commit(CipherCommit {
            expected: request.expected.to_string(),
            control: request.control.to_string(),
            controls: encode(&request.controls).map_err(storage)?,
            objects: ids.ids.clone(),
            remove: request.remove.iter().map(ToString::to_string).collect(),
            checkpoint: request.checkpoint.to_string(),
            baseline: request.baseline.to_string(),
            journal: encode(&request.journal).map_err(storage)?,
        });
        let view = view.map_err(storage)?;
        // An explicit remote rejection retains its category; an unusable success
        // reply cannot prove whether the mutation was durably published.
        self.accept(view)
            .map_err(|_| taypeer_storage::Error::CommitUncertain)
    }
    fn working_copy(&self) -> Digest {
        // Services only query this binding after snapshot authentication. This
        // mutex never runs platform/user callbacks while held, so it cannot poison.
        self.copy
            .lock()
            .expect("binding has no fallible callbacks")
            .expect("authenticated snapshot binds the working copy")
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), taypeer_storage::Error> {
        let ids = self.spool(vec![object.clone()]).map_err(storage)?;
        self.port.save_draft(ids.ids[0]).map_err(storage)
    }
    fn load_draft(
        &self,
        chain: &ControlChain,
    ) -> Result<Option<EncryptedObject>, taypeer_storage::Error> {
        self.port
            .load_draft()
            .map_err(storage)?
            .map(|id| {
                EncryptedObject::open_source(
                    descriptors::source(Arc::clone(&self.files), id),
                    chain,
                    descriptors::temporary(Arc::clone(&self.files)),
                )
            })
            .transpose()
    }
    fn discard_draft(&self) -> Result<(), taypeer_storage::Error> {
        self.port.discard_draft().map_err(storage)
    }
    fn path(&self) -> &Path {
        Path::new("")
    }
}

#[cfg(test)]
mod tests;
impl taypeer_runtime::platform_worker::PlatformDocument for RemoteDocument {
    fn portable_credentials(&self) -> bool {
        true
    }
    fn credential(
        &self,
        scope: &taypeer_storage::LocalCredentialScope,
        prepared: bool,
    ) -> Result<Option<Vec<u8>>, taypeer_runtime::RuntimeError> {
        let result = self
            .port
            .credential(encode(scope).map_err(recovery::remote_error)?, prepared)
            .map_err(recovery::remote_error)?;
        if result
            .as_ref()
            .is_some_and(|cipher| cipher.len() > 128 * 1024)
        {
            return Err(taypeer_runtime::RuntimeError::TooLarge);
        }
        Ok(result)
    }
    fn publish_credential(
        &self,
        scope: &taypeer_storage::LocalCredentialScope,
        cipher: &[u8],
        prepared: bool,
    ) -> Result<(), taypeer_runtime::RuntimeError> {
        self.port
            .publish_credential(
                encode(scope).map_err(recovery::remote_error)?,
                cipher.to_vec(),
                prepared,
            )
            .map_err(recovery::remote_error)
    }
    fn finalize_credential(
        &self,
        scope: &taypeer_storage::LocalCredentialScope,
        promote_prepared: bool,
    ) -> Result<(), taypeer_runtime::RuntimeError> {
        self.port
            .finalize_credential(
                encode(scope).map_err(recovery::remote_error)?,
                promote_prepared,
            )
            .map_err(recovery::remote_error)
    }
    fn activate_credentials(
        &self,
        capability: taypeer_runtime::profile::TransportCapability,
        authenticated: Option<Digest>,
    ) -> Result<(), taypeer_runtime::RuntimeError> {
        let bytes = zeroize::Zeroizing::new(encode(&capability).map_err(recovery::remote_error)?);
        self.port
            .activate_credentials(bytes.to_vec(), authenticated.map(|id| id.to_string()))
            .map_err(recovery::remote_error)
    }
    fn activate_recovery_credentials(
        &self,
        source: &taypeer_runtime::RecoverySource,
        capability: taypeer_runtime::profile::TransportCapability,
    ) -> Result<(), taypeer_runtime::RuntimeError> {
        let bytes = zeroize::Zeroizing::new(encode(&capability).map_err(recovery::remote_error)?);
        self.port
            .activate_recovery_credentials(
                encode(source).map_err(recovery::remote_error)?,
                bytes.to_vec(),
            )
            .map_err(recovery::remote_error)
    }
    fn selected_input(
        &self,
        id: u64,
    ) -> Result<Box<dyn std::io::Read + Send>, taypeer_runtime::RuntimeError> {
        Ok(Box::new(super::selected::RemoteInput {
            port: Arc::clone(&self.selected),
            id,
            offset: 0,
        }))
    }
    fn selected_output(
        &self,
        id: u64,
    ) -> Result<
        Box<dyn taypeer_runtime::platform_worker::SelectedOutput>,
        taypeer_runtime::RuntimeError,
    > {
        Ok(Box::new(super::selected::RemoteOutput {
            port: Arc::clone(&self.selected),
            id,
            offset: 0,
        }))
    }
    fn temporary(&self) -> TemporaryStorage {
        descriptors::temporary(Arc::clone(&self.files))
    }
    fn create(&self, seed: ArchiveSeed) -> Result<(), taypeer_runtime::RuntimeError> {
        let ids = self
            .spool(seed.objects)
            .map_err(|_| taypeer_runtime::RuntimeError::Transport)?;
        let result = self.port.create(CipherSeed {
            controls: encode(&seed.controls)
                .map_err(|_| taypeer_runtime::RuntimeError::Protocol)?,
            objects: ids.ids.clone(),
            checkpoint: seed.checkpoint.to_string(),
            baseline: seed.baseline.to_string(),
        });
        result.map_err(|error| taypeer_services::ServiceError::Storage(storage(error)).into())
    }
    fn freeze_recovery(
        &self,
        operation: Digest,
        authenticated: Digest,
    ) -> Result<(taypeer_runtime::RecoverySource, ArchiveSnapshot), taypeer_runtime::RuntimeError>
    {
        recovery::freeze(self, operation, authenticated)
    }
    fn recovery_bootstrap(
        &self,
        operation: Digest,
    ) -> Result<
        Option<taypeer_runtime::platform_worker::PlatformRecoveryBootstrap>,
        taypeer_runtime::RuntimeError,
    > {
        recovery::source(self, operation)
    }
    fn recovery_candidate(
        &self,
        intent: &taypeer_runtime::RecoveryIntent,
    ) -> Result<Option<ArchiveSnapshot>, taypeer_runtime::RuntimeError> {
        recovery::candidate(self, intent)
    }
    fn publish_recovery(
        &self,
        intent: &taypeer_runtime::RecoveryIntent,
        seed: ArchiveSeed,
    ) -> Result<taypeer_runtime::RecoveryPublication, taypeer_runtime::RuntimeError> {
        recovery::publish(self, intent, seed)
    }
    fn publish_recovery_credentials(
        &self,
        intent: &taypeer_runtime::RecoveryIntent,
        seed: ArchiveSeed,
        cipher: Vec<u8>,
        capability: taypeer_runtime::profile::TransportCapability,
    ) -> Result<taypeer_runtime::RecoveryPublication, taypeer_runtime::RuntimeError> {
        recovery::publish_credentials(self, intent, seed, cipher, capability)
    }
    fn confirm_recovery(
        &self,
        publication: &taypeer_runtime::RecoveryPublication,
    ) -> Result<(), taypeer_runtime::RuntimeError> {
        recovery::confirm(self, publication)
    }
}
