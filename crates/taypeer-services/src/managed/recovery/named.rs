//! Frozen named preparation. Its document and keys never follow the mutable source port.
use super::*;
use taypeer_core::DatabaseId;

/// Exact encrypted recovery receipt input; never a host/public-catalog DTO.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NamedRecoveryIntent {
    /// Explicit operation identity.
    pub operation: Digest,
    /// Logical source database, retained in the new trust set.
    pub database: DatabaseId,
    /// Profile-protected local source binding.
    pub working_copy: Digest,
    /// Authenticated original authority root.
    pub root: Digest,
    /// Exact immutable source ciphertext.
    pub source: Digest,
    /// Full local manager identity, including the transport role.
    pub identity: Identity,
    /// Exact user input, stored only under database encryption.
    pub name: String,
}
impl std::fmt::Debug for NamedRecoveryIntent {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("NamedRecoveryIntent { <redacted> }")
    }
}

/// One worker-owned plaintext preparation. Dropping it releases the frozen keyring and password.
/// No later service edit, incoming apply or mutable path is used to reconstruct its candidate.
pub struct TrustRecoveryPreparation {
    contents: RecoveryContents,
    intent: NamedRecoveryIntent,
    password: Zeroizing<Vec<u8>>,
}

/// Worker-owned authenticated source contents captured before any host callback.
/// It has no mutation/persistence methods and remains independent of later incoming changes.
pub struct TrustRecoverySource {
    contents: RecoveryContents,
    working_copy: Digest,
    timestamp: i64,
}
impl TrustRecoverySource {
    /// Validate exact form values independently of a possibly changed source password wrapper.
    /// Candidate retries authenticate their original encrypted receipt before checking first-attempt policy.
    pub fn validate_fields(name: &str, password: &[u8]) -> Result<(), ServiceError> {
        taypeer_core::validate_group_name(name).map_err(|_| ServiceError::InvalidInput)?;
        if password.is_empty() {
            return Err(ServiceError::InvalidInput);
        }
        Ok(())
    }
    /// Immutable ciphertext generation corresponding exactly to the retained plaintext contents.
    pub fn snapshot(&self) -> ArchiveSnapshot {
        self.contents.snapshot.clone()
    }
    /// Validate user input before any recovery publication or credential callback.
    pub fn validate_input(&self, name: &str, password: &[u8]) -> Result<(), ServiceError> {
        Self::validate_fields(name, password)?;
        if self
            .contents
            .snapshot
            .object(self.contents.accepted)?
            .unlock_key(password)
            .is_ok()
        {
            return Err(ServiceError::InvalidInput);
        }
        Ok(())
    }
    /// Bind one exact named request to these source contents without consulting the current service.
    pub fn bind_named(
        &self,
        intent: NamedRecoveryIntent,
        password: &[u8],
    ) -> Result<TrustRecoveryPreparation, ServiceError> {
        self.validate_input(&intent.name, password)?;
        intent.identity.validate()?;
        if intent.database != self.contents.snapshot.chain().head().database
            || intent.working_copy != self.working_copy
            || intent.root != self.contents.snapshot.chain().root()?
            || intent.source != self.contents.snapshot.fingerprint()
        {
            return Err(StorageError::Changed.into());
        }
        let mut contents = self.contents.clone();
        contents
            .document
            .set_writer(*intent.identity.device.as_bytes());
        let description = contents.document.description()?;
        contents
            .document
            .update_metadata_at(intent.name.clone(), description, self.timestamp)?;
        Ok(TrustRecoveryPreparation {
            contents,
            intent,
            password: Zeroizing::new(password.to_vec()),
        })
    }
}
impl TrustRecoveryPreparation {
    /// Compare the exact retry before doing provider, cryptographic or publication work.
    pub fn matches(&self, intent: &NamedRecoveryIntent, password: &[u8]) -> bool {
        self.intent == *intent && self.password.as_slice() == password
    }
    /// Prepare the frozen document with an author confined to the worker.
    /// The caller must retain the resulting encrypted seed on a pre-publication retry.
    pub fn prepare(&self, author: &AuthorKey) -> Result<ArchiveSeed, ServiceError> {
        if author.public() != self.intent.identity.author
            || author.device_id() != self.intent.identity.device
        {
            return Err(ServiceError::Unauthorized);
        }
        prepare_recovery_seed(
            &self.contents,
            &self.password,
            self.intent.identity,
            author,
            RecoveryReceipt {
                operation: self.intent.operation,
                source: self.intent.source,
                named: Some(self.intent.clone()),
            },
        )
    }
}

impl DatabaseService {
    /// Capture one authenticated document/checkpoint/control generation for local recovery only.
    pub fn freeze_trust_recovery_source(
        &self,
        session: &SessionToken,
    ) -> Result<TrustRecoverySource, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        compatibility::require_write(
            &managed
                .capabilities
                .assess(&managed.snapshot.chain().head().schema),
        )?;
        let current = managed.port.snapshot()?;
        if current.fingerprint() != managed.snapshot.fingerprint() {
            return Err(StorageError::Changed.into());
        }
        Ok(TrustRecoverySource {
            contents: RecoveryContents {
                snapshot: managed.snapshot.clone(),
                accepted: managed.accepted,
                document: state.document().clone(),
                metadata: open.metadata.clone(),
            },
            working_copy: managed.port.working_copy(),
            timestamp: (self.clock)(),
        })
    }
    /// Freeze a named recovery from the same authenticated archive/document generation.
    /// This changes neither source storage nor local forms and does not grant old-chain writes.
    pub fn freeze_named_trust_recovery(
        &self,
        session: &SessionToken,
        snapshot: ArchiveSnapshot,
        intent: NamedRecoveryIntent,
        password: &[u8],
    ) -> Result<TrustRecoveryPreparation, ServiceError> {
        let frozen = self.freeze_trust_recovery_source(session)?;
        if frozen.snapshot().fingerprint() != snapshot.fingerprint() {
            return Err(StorageError::Changed.into());
        }
        frozen.bind_named(intent, password)
    }
}

/// Authenticate the actual candidate against the complete encrypted named receipt.
/// Legacy receipts, another transport key/name/source or a changed candidate never authorize Android adoption.
pub fn validate_named_trust_recovery(
    source: &ArchiveSnapshot,
    candidate: &ArchiveSnapshot,
    intent: &NamedRecoveryIntent,
    password: &[u8],
) -> Result<Digest, ServiceError> {
    intent.identity.validate()?;
    let chain = candidate.chain();
    let checkpoint = candidate.object(candidate.metadata().manifest.body.checkpoint)?;
    let key = checkpoint.unlock_key(password)?;
    let (metadata, document) = codec::read_checkpoint(&checkpoint, &key, chain)?;
    validate_named_contents(source, candidate, intent, &metadata, &document)
}

/// Authenticate a restart candidate with its new password and protected source/identity.
/// No obsolete source password is required. Legacy and foreign-profile receipts refuse.
pub fn authenticate_named_trust_recovery(
    source: &ArchiveSnapshot,
    candidate: &ArchiveSnapshot,
    operation: Digest,
    working_copy: Digest,
    identity: Identity,
    password: &[u8],
) -> Result<NamedRecoveryIntent, ServiceError> {
    identity.validate()?;
    let checkpoint = candidate.object(candidate.metadata().manifest.body.checkpoint)?;
    let key = checkpoint.unlock_key(password)?;
    let (metadata, document) = codec::read_checkpoint(&checkpoint, &key, candidate.chain())?;
    let actual = metadata
        .recovery
        .receipt
        .as_ref()
        .and_then(|receipt| receipt.named.as_ref())
        .ok_or(taypeer_trust::Error::OperationMismatch)?;
    let expected = NamedRecoveryIntent {
        operation,
        database: source.chain().head().database.clone(),
        working_copy,
        root: source.chain().root()?,
        source: source.fingerprint(),
        identity,
        name: actual.name.clone(),
    };
    validate_named_contents(source, candidate, &expected, &metadata, &document)?;
    Ok(expected)
}

fn validate_named_contents(
    source: &ArchiveSnapshot,
    candidate: &ArchiveSnapshot,
    intent: &NamedRecoveryIntent,
    metadata: &Checkpoint,
    document: &Document,
) -> Result<Digest, ServiceError> {
    let chain = candidate.chain();
    let receipt = RecoveryReceipt {
        operation: intent.operation,
        source: intent.source,
        named: Some(intent.clone()),
    };
    if source.fingerprint() != intent.source
        || source.chain().root()? != intent.root
        || source.chain().head().database != intent.database
        || chain.head().database != intent.database
        || chain.head().manager != intent.identity.device
        || chain
            .head()
            .members
            .get(&intent.identity.device)
            .map(|member| member.identity)
            != Some(intent.identity)
        || chain.head().trust_set == source.chain().head().trust_set
        || metadata.recovery.receipt.as_ref() != Some(&receipt)
        || document.display_name()? != intent.name
    {
        return Err(taypeer_trust::Error::OperationMismatch.into());
    }
    Ok(chain.root()?)
}
