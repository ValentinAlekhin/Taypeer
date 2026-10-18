//! Recovery transfers verified ciphertext leases and bounded public receipts only.
use super::*;
use crate::document::recovery::{decode_public, encode_public, recovery_error};
use taypeer_runtime::{RecoveryIntent, RecoveryPublication, RecoverySource, RuntimeError};

/// One immutable encrypted recovery archive. It does not claim a registered working copy.
#[derive(uniffi::Record)]
pub struct CipherRecoveryArchive {
    /// Recipient-owned ciphertext descriptor lease.
    pub file: u64,
    /// Pinned signed trust root.
    pub root: String,
    /// Complete authenticated ciphertext generation.
    pub fingerprint: String,
}

/// Frozen public source and its exact separately streamed archive.
#[derive(uniffi::Record)]
pub struct CipherRecoverySource {
    /// Bounded typed public source, never name, password or plaintext document.
    pub source: Vec<u8>,
    /// Immutable encrypted archive; independent of the latest source path.
    pub archive: CipherRecoveryArchive,
    /// Actual encrypted candidate for password-only cold resume, never a registered-copy claim.
    pub candidate: Option<CipherRecoveryArchive>,
    /// Protected expected full identity/source binding; present exactly with a candidate.
    pub intent: Option<Vec<u8>>,
}

impl CipherWriter {
    fn recovery_view(
        &self,
        snapshot: ArchiveSnapshot,
    ) -> Result<CipherRecoveryArchive, AndroidError> {
        let root = snapshot.chain().root().map_err(failure)?.to_string();
        let fingerprint = snapshot.fingerprint().to_string();
        let file = self.files.allocate()?;
        let mut output = TableWrite {
            files: Arc::clone(&self.files),
            id: file,
            offset: 0,
        };
        if let Err(error) = snapshot.copy_ciphertext(&mut output) {
            self.files.release(file);
            return Err(failure(error));
        }
        Ok(CipherRecoveryArchive {
            file,
            root,
            fingerprint,
        })
    }
}

#[uniffi::export]
impl CipherWriter {
    /// Capture exactly the archive authenticated by this worker, or reconcile its prior source.
    pub fn freeze_recovery(
        &self,
        operation: String,
        authenticated: String,
    ) -> Result<CipherRecoverySource, AndroidError> {
        let (source, snapshot) = self
            .writer
            .freeze_recovery(digest(&operation)?, digest(&authenticated)?)
            .map_err(recovery_error)?;
        let source = encode_public(&source)?;
        let archive = self.recovery_view(snapshot)?;
        Ok(CipherRecoverySource {
            source,
            archive,
            candidate: None,
            intent: None,
        })
    }

    /// Reconcile a prior source before boot, never replacing it with the mutable latest file.
    pub fn recovery_source(
        &self,
        operation: String,
    ) -> Result<Option<CipherRecoverySource>, AndroidError> {
        self.writer
            .recovery_snapshot(digest(&operation)?)
            .map_err(recovery_error)?
            .map(|(source, snapshot)| {
                let resumed = self
                    .writer
                    .recovery_boot_candidate(&source)
                    .map_err(recovery_error)?;
                let source = encode_public(&source)?;
                let intent = resumed
                    .as_ref()
                    .map(|(intent, _)| encode_public(intent))
                    .transpose()?;
                if source.len() + intent.as_ref().map_or(0, Vec::len) > 64 * 1024 {
                    return Err(AndroidError::InvalidFile);
                }
                let archive = self.recovery_view(snapshot)?;
                let candidate = match resumed
                    .map(|(_, snapshot)| self.recovery_view(snapshot))
                    .transpose()
                {
                    Ok(candidate) => candidate,
                    Err(error) => {
                        self.files.release(archive.file);
                        return Err(error);
                    }
                };
                Ok(CipherRecoverySource {
                    source,
                    archive,
                    candidate,
                    intent,
                })
            })
            .transpose()
    }

    /// Transfer the actual encrypted candidate, retaining absence versus malformed/unreadable data.
    pub fn recovery_candidate(
        &self,
        intent: Vec<u8>,
    ) -> Result<Option<CipherRecoveryArchive>, AndroidError> {
        let intent: RecoveryIntent = decode_public(&intent)?;
        self.writer
            .recovery_candidate(&intent)
            .map_err(recovery_error)?
            .map(|snapshot| self.recovery_view(snapshot))
            .transpose()
    }

    /// Publish the independent genesis without adopting it or starting exchange.
    pub fn publish_recovery(
        &self,
        intent: Vec<u8>,
        seed: CipherSeed,
    ) -> Result<Vec<u8>, AndroidError> {
        // Take ownership of every incoming lease before decoding any metadata.
        let sources = self.sources(seed.objects)?;
        let intent: RecoveryIntent = decode_public(&intent)?;
        let controls: Vec<SignedControl> = decode(&seed.controls)?;
        let root = controls
            .first()
            .ok_or(AndroidError::InvalidFile)?
            .hash()
            .map_err(failure)?;
        let chain = ControlChain::validate(controls.clone(), root).map_err(failure)?;
        let seed = ArchiveSeed {
            controls,
            objects: self.objects(sources, &chain)?,
            checkpoint: digest(&seed.checkpoint)?,
            baseline: digest(&seed.baseline)?,
        };
        let result = self
            .writer
            .publish_recovery(&intent, seed)
            .map_err(recovery_error)?;
        // An unusable success reply cannot establish whether publication already happened.
        encode_public(&result).map_err(|_| AndroidError::CommitUncertain)
    }

    /// Persist only encrypted author credentials and the independent transport role.
    pub fn publish_recovery_credentials(
        &self,
        intent: Vec<u8>,
        seed: CipherSeed,
        cipher: Vec<u8>,
        capability: Vec<u8>,
    ) -> Result<Vec<u8>, AndroidError> {
        let sources = self.sources(seed.objects)?;
        let intent: RecoveryIntent = decode_public(&intent)?;
        let capability = zeroize::Zeroizing::new(capability);
        let capability = decode(&capability)?;
        if cipher.len() > 128 * 1024 {
            return Err(AndroidError::InvalidFile);
        }
        let controls: Vec<SignedControl> = decode(&seed.controls)?;
        let root = controls
            .first()
            .ok_or(AndroidError::InvalidFile)?
            .hash()
            .map_err(failure)?;
        let chain = ControlChain::validate(controls.clone(), root).map_err(failure)?;
        let seed = ArchiveSeed {
            controls,
            objects: self.objects(sources, &chain)?,
            checkpoint: digest(&seed.checkpoint)?,
            baseline: digest(&seed.baseline)?,
        };
        let result = self
            .writer
            .publish_recovery_with_credentials(&intent, seed, cipher, capability)
            .map_err(recovery_error)?;
        encode_public(&result).map_err(|_| AndroidError::CommitUncertain)
    }

    /// Recheck the candidate, registration, fsync and protected proof on every confirmation.
    pub fn confirm_recovery(&self, publication: Vec<u8>) -> Result<(), AndroidError> {
        let publication: RecoveryPublication = decode_public(&publication)?;
        self.writer
            .confirm_recovery(&publication)
            .map_err(recovery_error)
    }
}

pub(super) fn remote_error(error: AndroidError) -> RuntimeError {
    use taypeer_runtime::profile::ProfileError;
    match error {
        AndroidError::RecoveryRequiresNewOperation => RuntimeError::RecoveryRequiresNewOperation,
        AndroidError::ProfileBusy => ProfileError::Busy.into(),
        AndroidError::ProfileIo => ProfileError::Io.into(),
        AndroidError::ProfileInvalid => ProfileError::Invalid.into(),
        AndroidError::Credentials => ProfileError::Credentials.into(),
        other => taypeer_services::ServiceError::Storage(storage(other)).into(),
    }
}

fn accept_archive(
    remote: &RemoteDocument,
    view: CipherRecoveryArchive,
) -> Result<ArchiveSnapshot, AndroidError> {
    let source = descriptors::source(Arc::clone(&remote.files), view.file);
    accept_archive_source(remote, source, &view.root, &view.fingerprint)
}
fn accept_archive_source(
    remote: &RemoteDocument,
    source: taypeer_storage::CiphertextFile,
    root: &str,
    fingerprint: &str,
) -> Result<ArchiveSnapshot, AndroidError> {
    let snapshot = ArchiveSnapshot::from_source(
        source,
        Some(digest(root)?),
        descriptors::temporary(Arc::clone(&remote.files)),
    )
    .map_err(failure)?;
    if snapshot.fingerprint() != digest(fingerprint)? {
        return Err(AndroidError::InvalidFile);
    }
    Ok(snapshot)
}

pub(super) fn freeze(
    remote: &RemoteDocument,
    operation: Digest,
    authenticated: Digest,
) -> Result<(RecoverySource, ArchiveSnapshot), RuntimeError> {
    let view = remote
        .port
        .freeze_recovery(operation.to_string(), authenticated.to_string())
        .map_err(remote_error)?;
    let (source, snapshot) = accept_source(remote, operation, view)?;
    Ok((source, snapshot))
}

fn accept_source(
    remote: &RemoteDocument,
    operation: Digest,
    view: CipherRecoverySource,
) -> Result<(RecoverySource, ArchiveSnapshot), RuntimeError> {
    let resumed = accept_bootstrap(remote, operation, view)?;
    if resumed.candidate.is_some() || resumed.identity.is_some() {
        return Err(RuntimeError::Protocol);
    }
    Ok((resumed.source, resumed.original))
}

fn accept_bootstrap(
    remote: &RemoteDocument,
    operation: Digest,
    view: CipherRecoverySource,
) -> Result<taypeer_runtime::platform_worker::PlatformRecoveryBootstrap, RuntimeError> {
    // Own both leases before validating either archive or its metadata.
    let original_source = descriptors::source(Arc::clone(&remote.files), view.archive.file);
    let candidate_source = view
        .candidate
        .as_ref()
        .map(|view| descriptors::source(Arc::clone(&remote.files), view.file));
    if view.source.len() + view.intent.as_ref().map_or(0, Vec::len) > 64 * 1024 {
        return Err(RuntimeError::Protocol);
    }
    let snapshot = accept_archive_source(
        remote,
        original_source,
        &view.archive.root,
        &view.archive.fingerprint,
    )
    .map_err(remote_error)?;
    let source: RecoverySource = decode_public(&view.source).map_err(remote_error)?;
    if source.operation != operation
        || source.root
            != snapshot
                .chain()
                .root()
                .map_err(|_| RuntimeError::Protocol)?
        || source.fingerprint != snapshot.fingerprint()
        || source.database != snapshot.chain().head().database
    {
        return Err(RuntimeError::Protocol);
    }
    let (candidate, identity) = match (view.candidate, candidate_source, view.intent) {
        (None, None, None) => (None, None),
        (Some(view), Some(ciphertext), Some(bytes)) => {
            let intent: RecoveryIntent = decode_public(&bytes).map_err(remote_error)?;
            if intent.source != source {
                return Err(RuntimeError::Protocol);
            }
            let candidate =
                accept_archive_source(remote, ciphertext, &view.root, &view.fingerprint)
                    .map_err(remote_error)?;
            if candidate.chain().head().database != source.database
                || candidate
                    .chain()
                    .root()
                    .map_err(|_| RuntimeError::Protocol)?
                    == source.root
                || candidate.chain().head().manager != intent.identity.device
                || candidate
                    .chain()
                    .head()
                    .members
                    .get(&intent.identity.device)
                    .map(|member| member.identity)
                    != Some(intent.identity)
            {
                return Err(RuntimeError::Protocol);
            }
            (Some(candidate), Some(intent.identity))
        }
        _ => return Err(RuntimeError::Protocol),
    };
    let mut copy = remote.copy.lock().map_err(|_| RuntimeError::Transport)?;
    if copy.is_some_and(|prior| prior != source.working_copy) {
        return Err(RuntimeError::Protocol);
    }
    *copy = Some(source.working_copy);
    Ok(
        taypeer_runtime::platform_worker::PlatformRecoveryBootstrap {
            source,
            original: snapshot,
            candidate,
            identity,
        },
    )
}

pub(super) fn source(
    remote: &RemoteDocument,
    operation: Digest,
) -> Result<Option<taypeer_runtime::platform_worker::PlatformRecoveryBootstrap>, RuntimeError> {
    remote
        .port
        .recovery_source(operation.to_string())
        .map_err(remote_error)?
        .map(|view| accept_bootstrap(remote, operation, view))
        .transpose()
}

pub(super) fn candidate(
    remote: &RemoteDocument,
    intent: &RecoveryIntent,
) -> Result<Option<ArchiveSnapshot>, RuntimeError> {
    let bytes = encode_public(intent).map_err(remote_error)?;
    remote
        .port
        .recovery_candidate(bytes)
        .map_err(remote_error)?
        .map(|view| accept_archive(remote, view).map_err(remote_error))
        .transpose()
}

pub(super) fn publish(
    remote: &RemoteDocument,
    intent: &RecoveryIntent,
    seed: ArchiveSeed,
) -> Result<RecoveryPublication, RuntimeError> {
    let ids = remote.spool(seed.objects).map_err(remote_error)?;
    let reply = remote
        .port
        .publish_recovery(
            encode_public(intent).map_err(remote_error)?,
            CipherSeed {
                controls: encode(&seed.controls).map_err(remote_error)?,
                objects: ids.ids.clone(),
                checkpoint: seed.checkpoint.to_string(),
                baseline: seed.baseline.to_string(),
            },
        )
        .map_err(remote_error)?;
    let publication: RecoveryPublication =
        decode_public(&reply).map_err(|_| remote_error(AndroidError::CommitUncertain))?;
    if publication.intent != *intent {
        return Err(remote_error(AndroidError::CommitUncertain));
    }
    Ok(publication)
}

pub(super) fn publish_credentials(
    remote: &RemoteDocument,
    intent: &RecoveryIntent,
    seed: ArchiveSeed,
    cipher: Vec<u8>,
    capability: taypeer_runtime::profile::TransportCapability,
) -> Result<RecoveryPublication, RuntimeError> {
    let ids = remote.spool(seed.objects).map_err(remote_error)?;
    let capability = zeroize::Zeroizing::new(encode(&capability).map_err(remote_error)?);
    let reply = remote
        .port
        .publish_recovery_credentials(
            encode_public(intent).map_err(remote_error)?,
            CipherSeed {
                controls: encode(&seed.controls).map_err(remote_error)?,
                objects: ids.ids.clone(),
                checkpoint: seed.checkpoint.to_string(),
                baseline: seed.baseline.to_string(),
            },
            cipher,
            capability.to_vec(),
        )
        .map_err(remote_error)?;
    let publication: RecoveryPublication =
        decode_public(&reply).map_err(|_| remote_error(AndroidError::CommitUncertain))?;
    if publication.intent != *intent {
        return Err(remote_error(AndroidError::CommitUncertain));
    }
    Ok(publication)
}

pub(super) fn confirm(
    remote: &RemoteDocument,
    publication: &RecoveryPublication,
) -> Result<(), RuntimeError> {
    remote
        .port
        .confirm_recovery(encode_public(publication).map_err(remote_error)?)
        .map_err(remote_error)
}
