//! Serialized ciphertext recovery phases. User names, passwords and read keys never enter this owner.
use super::*;
use crate::{
    RecoveryIntent, RecoveryPhase, RecoveryPublication, RecoverySource,
    platform_worker::PlatformCipherWriter,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fs::File};
use taypeer_storage::PublicationMode;
use taypeer_trust::{AuthorKey, Identity};

const VERSION: u16 = 1;

mod writer_lifetimes;
pub(super) use writer_lifetimes::{LocalWriter, WriterLifetimes};

#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Attempt {
    version: u16,
    source: RecoverySource,
    source_path: PathBuf,
    owner: Digest,
    phase: RecoveryPhase,
    identity: Option<Identity>,
    publication: Option<RecoveryPublication>,
    // A proved-absent abandoned password attempt can never be reused with other inputs.
    #[serde(default, skip_serializing_if = "BTreeSet::is_empty")]
    refused: BTreeSet<Digest>,
}
fn mismatch() -> RuntimeError {
    taypeer_services::ServiceError::Trust(taypeer_trust::Error::OperationMismatch).into()
}
fn account(copy: Digest) -> String {
    format!("recovery-attempt:{copy}")
}
fn load(context: &HostContext, copy: Digest) -> Result<Option<Attempt>, RuntimeError> {
    let value: Option<Attempt> = context.profile.load_state(&account(copy))?;
    if let Some(value) = &value
        && (value.version != VERSION
            || value.source.working_copy != copy
            || !value.source_path.is_absolute())
    {
        return Err(crate::profile::ProfileError::Invalid.into());
    }
    if let Some(value) = &value {
        if value
            .identity
            .is_some_and(|identity| identity.validate().is_err())
            || value.publication.as_ref().is_some_and(|publication| {
                publication.intent.source != value.source
                    || Some(publication.intent.identity) != value.identity
                    || publication.root == value.source.root
            })
            || (matches!(
                value.phase,
                RecoveryPhase::CandidatePublished
                    | RecoveryPhase::ProofConfirmed
                    | RecoveryPhase::Adopted
            ) && value.publication.is_none())
            || (value.phase == RecoveryPhase::RefusedBeforePublication
                && (value.publication.is_some()
                    || !value.refused.contains(&value.source.operation)))
        {
            return Err(crate::profile::ProfileError::Invalid.into());
        }
    }
    Ok(value)
}
fn save(context: &HostContext, attempt: &Attempt) -> Result<(), RuntimeError> {
    // CredentialStore bounds the whole record to 64 KiB. Refused IDs are never silently evicted.
    context
        .profile
        .save_state(&account(attempt.source.working_copy), attempt)?;
    Ok(())
}
fn source_path(context: &HostContext, source: &RecoverySource) -> Result<PathBuf, RuntimeError> {
    let directory = context.profile.directory().join("recovery-sources");
    #[cfg(unix)]
    {
        use std::os::unix::fs::DirBuilderExt;
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .map_err(|_| RuntimeError::Transport)?;
    }
    #[cfg(not(unix))]
    std::fs::create_dir_all(&directory).map_err(|_| RuntimeError::Transport)?;
    let id = Digest::object(b"taypeer.recovery.source.v1", source)
        .map_err(|_| RuntimeError::Protocol)?;
    Ok(directory.join(format!("{id}.taypeer")))
}
fn destination(context: &HostContext, source: &RecoverySource) -> Result<PathBuf, RuntimeError> {
    let id = Digest::object(b"taypeer.recovery.destination.v1", source)
        .map_err(|_| RuntimeError::Protocol)?;
    let catalog = context
        .catalog
        .lock()
        .map_err(|_| RuntimeError::Transport)?;
    Ok(catalog.working_directory()?.join(format!("{id}.taypeer")))
}
fn sync(path: &Path) -> Result<(), RuntimeError> {
    File::open(path)
        .and_then(|file| file.sync_all())
        .and_then(|()| {
            File::open(path.parent().ok_or(std::io::ErrorKind::InvalidInput)?)
                .and_then(|file| file.sync_all())
        })
        .map_err(|_| cipher_ipc::storage(taypeer_storage::Error::CommitUncertain))
}
fn read(
    context: &HostContext,
    path: &Path,
    root: Option<Digest>,
) -> Result<Option<ArchiveSnapshot>, RuntimeError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(cipher_ipc::storage(taypeer_storage::Error::Io)),
    };
    Ok(Some(
        ArchiveSnapshot::from_file(file, root, context.ciphertext_staging())
            .map_err(cipher_ipc::storage)?,
    ))
}
fn check_source(attempt: &Attempt, source: &RecoverySource) -> Result<(), RuntimeError> {
    if attempt.refused.contains(&source.operation)
        || attempt.phase == RecoveryPhase::RefusedBeforePublication
    {
        return Err(RuntimeError::RecoveryRequiresNewOperation);
    }
    if attempt.source != *source {
        return Err(mismatch());
    }
    Ok(())
}
fn bind(
    context: &HostContext,
    attempt: &mut Attempt,
    intent: &RecoveryIntent,
) -> Result<(), RuntimeError> {
    check_source(attempt, &intent.source)?;
    intent
        .identity
        .validate()
        .map_err(|_| RuntimeError::Protocol)?;
    if intent.identity.transport != context.transport.public()
        || attempt.identity != Some(intent.identity)
    {
        return Err(mismatch());
    }
    save(context, attempt)
}
fn check_chain(intent: &RecoveryIntent, chain: &ControlChain) -> Result<Digest, RuntimeError> {
    let root = chain.root().map_err(|_| RuntimeError::Protocol)?;
    if chain.head().database != intent.source.database
        || root == intent.source.root
        || chain.head().manager != intent.identity.device
        || chain
            .head()
            .members
            .get(&intent.identity.device)
            .map(|member| member.identity)
            != Some(intent.identity)
    {
        return Err(mismatch());
    }
    Ok(root)
}
fn publication(
    intent: &RecoveryIntent,
    snapshot: &ArchiveSnapshot,
) -> Result<RecoveryPublication, RuntimeError> {
    let root = check_chain(intent, snapshot.chain())?;
    Ok(RecoveryPublication {
        intent: intent.clone(),
        root,
        fingerprint: snapshot.fingerprint(),
    })
}
fn receipt(
    context: &HostContext,
    attempt: &Attempt,
    value: &RecoveryPublication,
) -> Result<catalog::RecoveryReceipt, RuntimeError> {
    Ok(catalog::RecoveryReceipt {
        source: WorkingCopy {
            database: value.intent.source.database.clone(),
            root: value.intent.source.root,
            path: attempt.source_path.clone(),
        },
        destination: WorkingCopy {
            database: value.intent.source.database.clone(),
            root: value.root,
            path: destination(context, &value.intent.source)?,
        },
        operation: value.intent.source.operation,
        fingerprint: value.fingerprint,
        intent: Some(value.intent.clone()),
    })
}

impl PlatformCipherWriter {
    /// Select an existing protected ciphertext source before recovery password authentication.
    /// Normal document snapshots retain their current-generation policy.
    pub fn recovery_snapshot(
        &self,
        operation: Digest,
    ) -> Result<Option<(RecoverySource, ArchiveSnapshot)>, RuntimeError> {
        let _serial = self
            .context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let registration = self.registration()?;
        let Some(mut attempt) = load(&self.context, registration.working_copy)? else {
            return Ok(None);
        };
        if attempt.source.operation != operation {
            if attempt.refused.contains(&operation) {
                return Err(RuntimeError::RecoveryRequiresNewOperation);
            }
            if matches!(
                attempt.phase,
                RecoveryPhase::RefusedBeforePublication | RecoveryPhase::Adopted
            ) {
                return Ok(None);
            }
            return Err(cipher_ipc::storage(taypeer_storage::Error::Busy));
        }
        let source = &attempt.source;
        check_source(&attempt, source)?;
        if source.database != registration.database
            || source.root != registration.root
            || attempt.source_path != self.path
        {
            return Err(mismatch());
        }
        if attempt.owner != self.recovery_owner
            && read(&self.context, &destination(&self.context, source)?, None)?.is_none()
        {
            if attempt.publication.is_some() || attempt.phase != RecoveryPhase::SourceFrozen {
                return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
            }
            attempt.phase = RecoveryPhase::RefusedBeforePublication;
            attempt.refused.insert(operation);
            save(&self.context, &attempt)?;
            return Err(RuntimeError::RecoveryRequiresNewOperation);
        }
        let path = source_path(&self.context, &attempt.source)?;
        let snapshot =
            read(&self.context, &path, Some(attempt.source.root))?.ok_or_else(mismatch)?;
        if snapshot.fingerprint() != attempt.source.fingerprint {
            return Err(mismatch());
        }
        sync(&path)?;
        Ok(Some((attempt.source, snapshot)))
    }

    /// Read a restart candidate using the protected current profile identity, without author credentials.
    /// The worker must authenticate its named encrypted receipt before requesting the author capability.
    pub fn recovery_boot_candidate(
        &self,
        source: &RecoverySource,
    ) -> Result<Option<(RecoveryIntent, ArchiveSnapshot)>, RuntimeError> {
        let _serial = self
            .context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let attempt = self.recovery_attempt(source)?;
        let candidate = read(&self.context, &destination(&self.context, source)?, None)?;
        let Some(candidate) = candidate else {
            if attempt.publication.is_some() {
                return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
            }
            return Ok(None);
        };
        let identity = attempt.identity.ok_or_else(mismatch)?;
        if self.context.profile.identity()? != Some(identity) {
            return Err(mismatch());
        }
        let intent = RecoveryIntent {
            source: source.clone(),
            identity,
        };
        let actual = publication(&intent, &candidate)?;
        if attempt
            .publication
            .as_ref()
            .is_some_and(|prior| prior != &actual)
        {
            return Err(mismatch());
        }
        Ok(Some((intent, candidate)))
    }

    /// Persist an immutable authenticated source. Retries never consult the latest document generation.
    pub fn freeze_recovery(
        &self,
        operation: Digest,
        authenticated: Digest,
    ) -> Result<(RecoverySource, ArchiveSnapshot), RuntimeError> {
        let _serial = self
            .context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let registration = self.registration()?;
        let prior = load(&self.context, registration.working_copy)?;
        let mut source = RecoverySource {
            operation,
            database: registration.database,
            working_copy: registration.working_copy,
            root: registration.root,
            fingerprint: authenticated,
        };
        let mut attempt = match prior {
            Some(mut prior) if prior.source.operation == operation => {
                if prior.source.database != source.database
                    || prior.source.root != source.root
                    || prior.source.working_copy != source.working_copy
                    || prior.source_path != self.path
                {
                    return Err(mismatch());
                }
                source = prior.source.clone();
                check_source(&prior, &source)?;
                if authenticated != source.fingerprint {
                    return Err(mismatch());
                }
                if prior.owner != self.recovery_owner
                    && read(&self.context, &destination(&self.context, &source)?, None)?.is_none()
                {
                    if prior.publication.is_some() || prior.phase != RecoveryPhase::SourceFrozen {
                        return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
                    }
                    // A new worker cannot prove the vanished password/name attempt from a source-only receipt.
                    prior.phase = RecoveryPhase::RefusedBeforePublication;
                    prior.refused.insert(operation);
                    save(&self.context, &prior)?;
                    return Err(RuntimeError::RecoveryRequiresNewOperation);
                }
                prior
            }
            prior => {
                let refused = if let Some(prior) = prior {
                    if !matches!(
                        prior.phase,
                        RecoveryPhase::RefusedBeforePublication | RecoveryPhase::Adopted
                    ) {
                        return Err(cipher_ipc::storage(taypeer_storage::Error::Busy));
                    }
                    prior.refused
                } else {
                    BTreeSet::new()
                };
                if refused.contains(&operation) {
                    return Err(RuntimeError::RecoveryRequiresNewOperation);
                }
                Attempt {
                    version: VERSION,
                    source: source.clone(),
                    source_path: self.path.clone(),
                    owner: self.recovery_owner,
                    phase: RecoveryPhase::SourceFrozen,
                    identity: None,
                    publication: None,
                    refused,
                }
            }
        };
        let path = source_path(&self.context, &source)?;
        let snapshot = if let Some(snapshot) = read(&self.context, &path, Some(source.root))? {
            if snapshot.fingerprint() != source.fingerprint {
                return Err(mismatch());
            }
            snapshot
        } else {
            let mut held = self
                .recovery_lease
                .lock()
                .map_err(|_| RuntimeError::Transport)?;
            if held.is_none() {
                let issued = self
                    .authentication
                    .lock()
                    .map_err(|_| RuntimeError::Transport)?
                    .take()
                    .ok_or_else(mismatch)?;
                if issued.fingerprint() != source.fingerprint
                    || issued.chain().root().map_err(|_| RuntimeError::Protocol)? != source.root
                {
                    return Err(mismatch());
                }
                *held = Some(issued);
            }
            let issued = held.as_ref().ok_or_else(mismatch)?;
            if issued.fingerprint() != source.fingerprint {
                return Err(mismatch());
            }
            // Protect the original operation before any source-file publication; all future reads reconcile it.
            save(&self.context, &attempt)?;
            let mut temporary = NamedTempFile::new_in(path.parent().ok_or(RuntimeError::Protocol)?)
                .map_err(|_| RuntimeError::Transport)?;
            issued
                .copy_ciphertext(temporary.as_file_mut())
                .map_err(cipher_ipc::storage)?;
            taypeer_storage::publish_file(temporary, &path, PublicationMode::Create)
                .map_err(cipher_ipc::storage)?;
            read(&self.context, &path, Some(source.root))?.ok_or_else(mismatch)?
        };
        sync(&path)?;
        attempt.owner = self.recovery_owner;
        save(&self.context, &attempt)?;
        Ok((source, snapshot))
    }

    /// Issue an author only to the private exact recovery generation, including a readable frozen fork.
    pub fn author_recovery(&self, source: &RecoverySource) -> Result<AuthorKey, RuntimeError> {
        let _serial = self
            .context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let mut attempt = self.recovery_attempt(source)?;
        if attempt.owner != self.recovery_owner {
            return Err(mismatch());
        }
        let snapshot = read(
            &self.context,
            &source_path(&self.context, source)?,
            Some(source.root),
        )?
        .ok_or_else(mismatch)?;
        if snapshot.fingerprint() != source.fingerprint {
            return Err(mismatch());
        }
        let author = self.context.profile.author()?;
        let identity = Identity::new(author.public(), self.context.transport.public())
            .map_err(|_| RuntimeError::Protocol)?;
        if attempt.identity.is_some_and(|prior| prior != identity) {
            return Err(mismatch());
        }
        attempt.identity = Some(identity);
        bind(
            &self.context,
            &mut attempt,
            &RecoveryIntent {
                source: source.clone(),
                identity,
            },
        )?;
        Ok(author)
    }

    /// Read the real candidate after binding the current full public identity.
    pub fn recovery_candidate(
        &self,
        intent: &RecoveryIntent,
    ) -> Result<Option<ArchiveSnapshot>, RuntimeError> {
        let _serial = self
            .context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let mut attempt = self.recovery_attempt(&intent.source)?;
        bind(&self.context, &mut attempt, intent)?;
        let value = read(
            &self.context,
            &destination(&self.context, &intent.source)?,
            None,
        )?;
        if let Some(value) = &value {
            let actual = publication(intent, value)?;
            if attempt
                .publication
                .as_ref()
                .is_some_and(|prior| prior != &actual)
            {
                return Err(mismatch());
            }
        }
        Ok(value)
    }

    /// Publish the exact encrypted seed separately. No writer or network is attached to it.
    pub fn publish_recovery(
        &self,
        intent: &RecoveryIntent,
        seed: ArchiveSeed,
    ) -> Result<RecoveryPublication, RuntimeError> {
        let _serial = self
            .context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let mut attempt = self.recovery_attempt(&intent.source)?;
        bind(&self.context, &mut attempt, intent)?;
        let path = destination(&self.context, &intent.source)?;
        if read(&self.context, &path, None)?.is_some() {
            return Err(cipher_ipc::storage(taypeer_storage::Error::AlreadyExists));
        }
        let root = seed
            .controls
            .first()
            .ok_or(RuntimeError::Protocol)?
            .hash()
            .map_err(|_| RuntimeError::Protocol)?;
        let chain = ControlChain::validate(seed.controls.clone(), root)
            .map_err(|_| RuntimeError::Protocol)?;
        check_chain(intent, &chain)?;
        self.context.publish_recovery(&path, seed)?;
        let snapshot = read(&self.context, &path, None)?.ok_or_else(mismatch)?;
        let value = publication(intent, &snapshot)?;
        attempt.publication = Some(value.clone());
        attempt.phase = RecoveryPhase::CandidatePublished;
        save(&self.context, &attempt)?;
        Ok(value)
    }

    /// Confirm actual candidate durability and protected proof before catalog adoption.
    pub fn confirm_recovery(&self, value: &RecoveryPublication) -> Result<(), RuntimeError> {
        let _serial = self
            .context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let mut attempt = self.recovery_attempt(&value.intent.source)?;
        bind(&self.context, &mut attempt, &value.intent)?;
        if attempt
            .publication
            .as_ref()
            .is_some_and(|prior| prior != value)
        {
            return Err(mismatch());
        }
        let path = destination(&self.context, &value.intent.source)?;
        let snapshot = read(&self.context, &path, Some(value.root))?.ok_or_else(mismatch)?;
        if publication(&value.intent, &snapshot)? != *value {
            return Err(mismatch());
        }
        let registration = self
            .context
            .profile
            .registration(&path)?
            .ok_or_else(mismatch)?;
        if registration.database != value.intent.source.database || registration.root != value.root
        {
            return Err(mismatch());
        }
        sync(&path)?;
        let proof = receipt(&self.context, &attempt, value)?;
        let key = proof.account()?;
        if self
            .context
            .profile
            .load_state::<catalog::RecoveryReceipt>(&key)?
            .is_some_and(|prior| prior != proof)
        {
            return Err(mismatch());
        }
        self.context.profile.save_state(&key, &proof)?;
        let mut catalog = self
            .context
            .catalog
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        catalog.reconcile()?;
        catalog.record_recovery(proof)?;
        attempt.publication = Some(value.clone());
        attempt.phase = RecoveryPhase::ProofConfirmed;
        save(&self.context, &attempt)
    }

    fn recovery_attempt(&self, source: &RecoverySource) -> Result<Attempt, RuntimeError> {
        let registration = self.registration()?;
        let attempt = load(&self.context, source.working_copy)?.ok_or_else(mismatch)?;
        check_source(&attempt, source)?;
        if registration.database != source.database
            || registration.root != source.root
            || registration.working_copy != source.working_copy
            || attempt.source_path != self.path
        {
            return Err(mismatch());
        }
        Ok(attempt)
    }
}

impl RuntimeHost {
    /// Inspect a protected operation without decrypting the archive. Missing, corrupt and unavailable differ.
    /// Reported phases are reconciled against actual ciphertext; this never asserts writer closure.
    pub fn recovery_phase(
        &self,
        source: &RecoverySource,
    ) -> Result<Option<RecoveryPhase>, RuntimeError> {
        let context = self.context.for_database(&source.database)?;
        let _serial = context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let Some(mut attempt) = load(&context, source.working_copy)? else {
            return Ok(None);
        };
        if attempt.source != *source {
            return Err(mismatch());
        }
        if attempt.phase == RecoveryPhase::RefusedBeforePublication {
            return Ok(Some(attempt.phase));
        }
        let path = source_path(&context, source)?;
        let snapshot = read(&context, &path, Some(source.root))?.ok_or_else(mismatch)?;
        if snapshot.fingerprint() != source.fingerprint {
            return Err(mismatch());
        }
        sync(&path)?;
        if let Some(identity) = attempt.identity {
            let intent = RecoveryIntent {
                source: source.clone(),
                identity,
            };
            let candidate_path = destination(&context, source)?;
            if let Some(candidate) = read(&context, &candidate_path, None)? {
                let actual = publication(&intent, &candidate)?;
                if attempt
                    .publication
                    .as_ref()
                    .is_some_and(|prior| prior != &actual)
                {
                    return Err(mismatch());
                }
                sync(&candidate_path)?;
                if attempt.phase == RecoveryPhase::SourceFrozen {
                    attempt.publication = Some(actual);
                    attempt.phase = RecoveryPhase::CandidatePublished;
                    save(&context, &attempt)?;
                }
            } else if attempt.publication.is_some() {
                return Err(cipher_ipc::storage(taypeer_storage::Error::Changed));
            }
        }
        if let Some(publication) = &attempt.publication {
            let proof = receipt(&context, &attempt, publication)?;
            proof.verify_destination()?;
            if matches!(
                attempt.phase,
                RecoveryPhase::ProofConfirmed | RecoveryPhase::Adopted
            ) {
                self.verify_recovery_proof(&proof)?;
            }
        }
        Ok(Some(attempt.phase))
    }
    /// Restart-visible protected operations for existing catalog sources.
    /// Missing records are omitted; corrupt or unavailable state is never replaced by an empty list.
    pub fn pending_recoveries(&self) -> Result<Vec<crate::PendingRecovery>, RuntimeError> {
        {
            let mut catalog = self.catalog.lock().map_err(|_| RuntimeError::Transport)?;
            catalog.reconcile()?;
        }
        let copies = self.working_copies()?;
        let mut pending = Vec::new();
        for copy in copies {
            let context = self.context.for_database(&copy.database)?;
            let Some(registration) = context.profile.registration(&copy.path)? else {
                return Err(RuntimeError::Protocol);
            };
            let source = {
                let _serial = context
                    .recovery
                    .lock()
                    .map_err(|_| RuntimeError::Transport)?;
                let Some(attempt) = load(&context, registration.working_copy)? else {
                    continue;
                };
                if attempt.phase == RecoveryPhase::Adopted {
                    continue;
                }
                if attempt.source_path != copy.path
                    || attempt.source.root != copy.root
                    || attempt.source.database != copy.database
                {
                    return Err(mismatch());
                }
                attempt.source
            };
            let phase = self
                .recovery_phase(&source)?
                .ok_or(RuntimeError::Protocol)?;
            pending.push(crate::PendingRecovery {
                source,
                working_copy: copy,
                phase,
            });
        }
        Ok(pending)
    }
    /// Adopt only an exact protected recovery after all source processes and writers actually close.
    pub fn adopt_platform_recovery(
        &self,
        value: &RecoveryPublication,
    ) -> Result<WorkingCopy, RuntimeError> {
        let context = self.context.for_database(&value.intent.source.database)?;
        let _serial = context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let (attempt, proof) = self.verified_platform_recovery(&context, value)?;
        self.require_closed_recovery_source(&value.intent.source.database)?;
        self.publish_platform_adoption(&context, attempt, proof)
    }

    /// Complete platform recovery after actual source processes and writer capabilities close.
    /// Exact protected proof is required before unregistering the source ciphertext coordinator.
    /// A live, reopening or locally held writer returns `Storage(Busy)` without removing it;
    /// ordinary `adopt_platform_recovery` retains its explicit-registration closure contract.
    pub fn adopt_closed_platform_recovery(
        &self,
        value: &RecoveryPublication,
    ) -> Result<WorkingCopy, RuntimeError> {
        let context = self.context.for_database(&value.intent.source.database)?;
        let _serial = context
            .recovery
            .lock()
            .map_err(|_| RuntimeError::Transport)?;
        let (attempt, proof) = self.verified_platform_recovery(&context, value)?;
        let source_path = proof.source.path.clone();
        context
            .recovery_writers
            .with_closed_source(&source_path, || {
                self.require_platform_source_closure(value, &proof)?;
                self.close(&value.intent.source.database)?;
                // Retain the general guard, including registrations in other host contexts.
                self.require_closed_recovery_source(&value.intent.source.database)?;
                self.publish_platform_adoption(&context, attempt, proof)
            })
    }

    fn verified_platform_recovery(
        &self,
        context: &HostContext,
        value: &RecoveryPublication,
    ) -> Result<(Attempt, catalog::RecoveryReceipt), RuntimeError> {
        let attempt = load(context, value.intent.source.working_copy)?.ok_or_else(mismatch)?;
        check_source(&attempt, &value.intent.source)?;
        if attempt.publication.as_ref() != Some(value)
            || !matches!(
                attempt.phase,
                RecoveryPhase::ProofConfirmed | RecoveryPhase::Adopted
            )
        {
            return Err(mismatch());
        }
        let proof = receipt(context, &attempt, value)?;
        self.verify_recovery_proof(&proof)?;
        Ok((attempt, proof))
    }

    fn require_platform_source_closure(
        &self,
        value: &RecoveryPublication,
        proof: &catalog::RecoveryReceipt,
    ) -> Result<(), RuntimeError> {
        let source = &value.intent.source;
        if self.sessions.statuses().iter().any(|status| {
            (status.database.is_none() || status.database.as_ref() == Some(&source.database))
                && status.phase != crate::session::SessionPhase::Closed
        }) {
            return Err(cipher_ipc::storage(taypeer_storage::Error::Busy));
        }
        for context in self.context.active_contexts()? {
            let copies = context.copies.lock().map_err(|_| RuntimeError::Transport)?;
            for (path, registration) in copies.iter() {
                if registration.database == source.database
                    && (path != &proof.source.path
                        || registration.root != source.root
                        || registration.working_copy != source.working_copy)
                {
                    return Err(cipher_ipc::storage(taypeer_storage::Error::Busy));
                }
            }
        }
        Ok(())
    }

    fn publish_platform_adoption(
        &self,
        context: &HostContext,
        mut attempt: Attempt,
        proof: catalog::RecoveryReceipt,
    ) -> Result<WorkingCopy, RuntimeError> {
        let mut catalog = self.catalog.lock().map_err(|_| RuntimeError::Transport)?;
        catalog.reconcile()?;
        catalog.adopt_recovery(&proof)?;
        attempt.phase = RecoveryPhase::Adopted;
        save(context, &attempt)?;
        Ok(proof.destination)
    }
}

#[cfg(test)]
mod tests;
