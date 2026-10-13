//! Unlocked authority, original-source admission and encrypted candidate preparation.
//! The persistence port owns neither read keys nor document/author state.
use super::*;
use std::collections::BTreeSet;
use taypeer_core::{BlobId, DatabasePolicy};
use taypeer_document::OriginalChange;
use taypeer_storage::{
    ArchiveSeed, ArchiveSnapshot, CipherPersistence, EncryptedObject, PreparedCommit,
};
use taypeer_trust::{AuthorKey, ControlChain, Digest, Identity, ObjectKind, SourceProof};

mod administration;
mod management;
pub use management::OpenManagementBasis;
mod open;
pub use open::ManagedKeyScope;
mod apply;
mod codec;
mod collection;
pub(crate) mod compatibility;
pub use collection::CollectionReport;
mod pending;
mod policy;
pub use policy::{DatabasePolicyDetails, ProtectionInfo};
mod recovery;
pub use apply::{ApplyReport, PendingPacket, PendingReason};
use codec::Checkpoint;
pub use pending::ReceivedSource;
pub use recovery::{
    NamedRecoveryIntent, TrustRecoveryPreparation, TrustRecoverySource,
    authenticate_named_trust_recovery, validate_named_trust_recovery,
};

#[cfg(test)]
mod tests;

/// Stages of the local credential transaction surrounding a database epoch commit.
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum EpochCredentialStage {
    /// Durably prepare the new object and marker before committing the database.
    Prepare,
    /// Publish the new object and final marker after the database commit.
    Finalize,
}

impl From<taypeer_trust::Error> for ServiceError {
    fn from(error: taypeer_trust::Error) -> Self {
        Self::Trust(error)
    }
}

pub(super) struct ManagedState {
    pub port: Box<dyn CipherPersistence>,
    pub snapshot: ArchiveSnapshot,
    baseline: Digest,
    accepted: Digest,
    // Reconciliation can install acknowledged state before a later apply attempt fails.
    pub(super) reconciled_application: bool,
    open: Option<Unlocked>,
    capabilities: taypeer_core::ClientCapabilities,
    pending_management: Option<management::PendingTransfer>,
}
struct PreparedPublication {
    request: PreparedCommit,
    metadata: Checkpoint,
    header: Option<Vec<u8>>,
}
struct Unlocked {
    author: Option<AuthorKey>,
    header: Vec<u8>,
    metadata: Checkpoint,
    control: Digest,
    authentication: open::AuthenticationProvenance,
    password_kdf: Option<policy::PasswordKdfMeasurement>,
}
impl ManagedState {
    pub fn policy(&self) -> DatabasePolicy {
        self.open
            .as_ref()
            .map_or_else(Default::default, |open| open.metadata.policy)
    }
    pub fn close(&mut self) {
        self.open = None;
        self.pending_management = None;
    }
    pub fn check_edit_permission(&self) -> Result<(), ServiceError> {
        if self.pending_management.is_some() {
            return Err(StorageError::CommitUncertain.into());
        }
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        self.check_format_write()?;
        let author = open.author.as_ref().ok_or(ServiceError::ReadOnly)?;
        let current = self.port.snapshot()?;
        if !current.metadata().journal.forks.is_empty() {
            return Err(taypeer_trust::Error::Fork.into());
        }
        if !current
            .chain()
            .head()
            .members
            .contains_key(&author.device_id())
        {
            return Err(ServiceError::ReadOnly);
        }
        Ok(())
    }
    pub fn check_session(&self) -> Result<(), ServiceError> {
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let latest = self.port.snapshot()?;
        compatibility::require_read(&self.capabilities.assess(&latest.chain().head().schema))?;
        if latest.chain().head().epoch != self.snapshot.chain().at(open.control)?.epoch {
            return Err(ServiceError::ExpiredSession);
        }
        if let Some(author) = &open.author {
            let was_admitted = self
                .snapshot
                .chain()
                .at(open.control)?
                .members
                .contains_key(&author.device_id());
            if was_admitted
                && !latest
                    .chain()
                    .continuous(author.device_id(), open.control)?
            {
                return Err(ServiceError::ExpiredSession);
            }
        }
        Ok(())
    }
    fn writer(&self) -> Result<&AuthorKey, ServiceError> {
        if self.pending_management.is_some() {
            return Err(StorageError::CommitUncertain.into());
        }
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        self.check_format_write()?;
        let key = open.author.as_ref().ok_or(ServiceError::ReadOnly)?;
        if !self.snapshot.metadata().journal.forks.is_empty() {
            return Err(taypeer_trust::Error::Fork.into());
        }
        if !self
            .snapshot
            .chain()
            .head()
            .members
            .contains_key(&key.device_id())
        {
            return Err(ServiceError::ReadOnly);
        }
        let current = self.port.snapshot()?;
        if current.chain().head_hash()? != self.snapshot.chain().head_hash()? {
            return Err(ServiceError::ExpiredSession);
        }
        if current.fingerprint() != self.snapshot.fingerprint() {
            return Err(StorageError::Changed.into());
        }
        Ok(key)
    }
    pub fn commit(&mut self, document: &Document, blobs: &BlobStore) -> Result<(), ServiceError> {
        self.commit_bound(document, blobs, &BTreeMap::new())
    }
    fn commit_bound(
        &mut self,
        document: &Document,
        blobs: &BlobStore,
        bindings: &BTreeMap<BlobId, Digest>,
    ) -> Result<(), ServiceError> {
        let author = self.writer()?;
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let chain = self.snapshot.chain();
        if open.control != chain.head_hash()? {
            return Err(ServiceError::AwaitingData);
        }
        let key = open.metadata.key(chain.head().epoch)?;
        let mut metadata = open.metadata.clone();
        apply::merge_bindings(&mut metadata.blobs, bindings)?;
        let mut objects = self.seal_new_blobs(blobs, &mut metadata)?;
        for source in document.changes_since(&[])? {
            if let Some(proof) = metadata.proofs.get(&source.metadata().hash) {
                metadata.verify_original(&source, proof, chain)?;
                continue;
            }
            if source.metadata().author != Some(*author.device_id().as_bytes()) {
                return Err(ServiceError::Unauthorized);
            }
            let proof =
                SourceProof::sign_bound(chain, author, source.bytes(), metadata.blobs.clone())?;
            let object = codec::seal_payload(
                chain,
                author,
                ObjectKind::Change,
                &open.header,
                &key,
                &proof,
                source.bytes(),
                self.snapshot.temporary_storage(),
            )?;
            metadata.processed.insert(object.descriptor().digest);
            objects.push(object);
            metadata
                .proofs
                .insert(source.metadata().hash.clone(), proof);
        }
        self.persist(document, metadata, objects, None)
    }
    fn seal_new_blobs(
        &self,
        blobs: &BlobStore,
        metadata: &mut Checkpoint,
    ) -> Result<Vec<EncryptedObject>, ServiceError> {
        let author = self.writer()?;
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let chain = self.snapshot.chain();
        let key = metadata.key(chain.head().epoch)?;
        let mut objects = Vec::new();
        for id in blobs.ids() {
            if metadata.blobs.contains_key(id) {
                continue;
            }
            let selected = blobs.retained(&BTreeSet::from([id.clone()]));
            let object = codec::seal_bundle(
                chain,
                author,
                ObjectKind::Blob,
                &open.header,
                &key,
                &[],
                &selected,
            )?;
            metadata
                .blobs
                .insert(id.clone(), object.descriptor().digest);
            objects.push(object);
        }
        Ok(objects)
    }
    fn persist(
        &mut self,
        document: &Document,
        metadata: Checkpoint,
        objects: Vec<EncryptedObject>,
        administration: Option<administration::Transition>,
    ) -> Result<(), ServiceError> {
        self.persist_candidate(document, metadata, objects, administration, None)
    }
    fn persist_candidate(
        &mut self,
        document: &Document,
        metadata: Checkpoint,
        objects: Vec<EncryptedObject>,
        administration: Option<administration::Transition>,
        collection: Option<collection::Collection>,
    ) -> Result<(), ServiceError> {
        let candidate =
            self.prepare_publication(document, metadata, objects, administration, collection)?;
        let snapshot = self.port.commit(candidate.request.clone())?;
        self.accept_publication(candidate, snapshot)
    }
    fn prepare_publication(
        &self,
        document: &Document,
        mut metadata: Checkpoint,
        mut objects: Vec<EncryptedObject>,
        administration: Option<administration::Transition>,
        collection: Option<collection::Collection>,
    ) -> Result<PreparedPublication, ServiceError> {
        let author = self.writer()?;
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let (chain, header, mut journal) = match &administration {
            Some(transition) => (
                &transition.chain,
                transition.header.as_slice(),
                transition.journal.clone(),
            ),
            None => (
                self.snapshot.chain(),
                open.header.as_slice(),
                self.snapshot.metadata().journal.clone(),
            ),
        };
        if let Some(collection) = &collection {
            journal = collection.journal.clone();
        }
        let key = metadata.key(chain.head().epoch)?;
        metadata.processed.insert(self.accepted);
        if let Some(collection) = &collection {
            metadata
                .processed
                .retain(|id| !collection.remove.contains(id));
        }
        let clear = Zeroizing::new(document.export());
        metadata.verify(document, chain, chain.head_hash()?)?;
        let checkpoint = codec::seal_payload(
            chain,
            author,
            ObjectKind::Checkpoint,
            header,
            &key,
            &metadata,
            &clear,
            self.snapshot.temporary_storage(),
        )?;
        let checkpoint_id = checkpoint.descriptor().digest;
        objects.push(checkpoint);
        let baseline = if administration.is_some()
            || collection.as_ref().is_some_and(|c| c.refresh_baseline)
        {
            let object = codec::seal_payload(
                chain,
                author,
                ObjectKind::Baseline,
                header,
                &key,
                &metadata,
                &clear,
                self.snapshot.temporary_storage(),
            )?;
            let id = object.descriptor().digest;
            objects.push(object);
            journal
                .retained
                .insert(self.snapshot.metadata().manifest.body.checkpoint);
            id
        } else {
            self.baseline
        };
        let remove = collection.map_or_else(BTreeSet::new, |c| c.remove);
        journal.retained.retain(|id| !remove.contains(id));
        let request = PreparedCommit {
            expected: self.snapshot.fingerprint(),
            control: self.snapshot.chain().head_hash()?,
            controls: chain.records().to_vec(),
            objects,
            remove,
            checkpoint: checkpoint_id,
            baseline,
            journal,
        };
        Ok(PreparedPublication {
            request,
            metadata,
            header: administration.map(|transition| transition.header),
        })
    }
    fn accept_publication(
        &mut self,
        candidate: PreparedPublication,
        snapshot: ArchiveSnapshot,
    ) -> Result<(), ServiceError> {
        let open = self.open.as_mut().ok_or(ServiceError::Locked)?;
        if let Some(header) = candidate.header {
            open.replace_header(header);
        }
        open.metadata = candidate.metadata;
        open.control = snapshot.chain().head_hash()?;
        self.baseline = candidate.request.baseline;
        self.accepted = candidate.request.checkpoint;
        self.snapshot = snapshot;
        Ok(())
    }
    pub fn save_draft(
        &self,
        draft: &DraftCollection,
        blobs: &BlobStore,
    ) -> Result<(), ServiceError> {
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let Some(author) = &open.author else {
            return Err(ServiceError::ReadOnly);
        };
        let mut interrupted = draft.clone();
        interrupted.interrupt();
        let clear = Zeroizing::new(
            serde_json::to_vec(&(self.port.working_copy(), interrupted))
                .map_err(|_| ServiceError::InvalidDocument)?,
        );
        let selected = blobs.retained(&draft.binary_references());
        let object = codec::seal_bundle(
            self.snapshot.chain(),
            author,
            ObjectKind::LocalDraft,
            &open.header,
            &open.metadata.key(self.snapshot.chain().head().epoch)?,
            &clear,
            &selected,
        )?;
        self.port.save_draft(&object)?;
        Ok(())
    }

    pub(super) fn reload_confirmed(&mut self) -> Result<(Document, BlobStore), ServiceError> {
        self.check_session()?;
        self.check_edit_permission()?;
        let snapshot = self.port.snapshot()?;
        let checkpoint = snapshot.object(snapshot.metadata().manifest.body.checkpoint)?;
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let (metadata, mut document) = codec::read_checkpoint(
            &checkpoint,
            &open.metadata.key(checkpoint.envelope().epoch)?,
            snapshot.chain(),
        )?;
        let author = open.author.as_ref().ok_or(ServiceError::ReadOnly)?;
        if checkpoint.envelope().author != author.device_id() {
            return Err(ServiceError::InvalidContext);
        }
        document.set_writer(*author.device_id().as_bytes());
        let mut blobs = BlobStore::with_temporary_storage(snapshot.temporary_storage())?;
        apply::load_blobs(&snapshot, &metadata, &document, &mut blobs)?;
        let header = checkpoint.password_header()?;
        let open = self.open.as_mut().ok_or(ServiceError::Locked)?;
        open.metadata = metadata;
        open.replace_header(header);
        open.control = checkpoint.envelope().control;
        self.reconciled_application |= self.baseline != snapshot.metadata().manifest.body.baseline;
        self.baseline = snapshot.metadata().manifest.body.baseline;
        self.accepted = checkpoint.descriptor().digest;
        self.snapshot = snapshot;
        Ok((document, blobs))
    }
    fn load_draft(&self, blobs: &mut BlobStore) -> Result<DraftCollection, ServiceError> {
        let Some(object) = self.port.load_draft(self.snapshot.chain())? else {
            return Ok(DraftCollection::new(
                self.snapshot.chain().head().database.clone(),
            ));
        };
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let (clear, staged) = object.unlock_bundle(&open.metadata.key(object.envelope().epoch)?)?;
        let (working_copy, draft): (Digest, DraftCollection) =
            serde_json::from_slice(&clear).map_err(|_| ServiceError::InvalidDocument)?;
        if working_copy != self.port.working_copy()
            || draft
                .binary_references()
                .iter()
                .any(|id| staged.length(id).is_none())
        {
            return Err(ServiceError::InvalidContext);
        }
        draft.validate(&self.snapshot.chain().head().database)?;
        blobs.import(&staged)?;
        Ok(draft)
    }
}

impl DatabaseService {
    /// Prepare a new signed database in the unlocked worker. The result contains only
    /// ciphertext; the coordinator must persist it before reporting creation success.
    pub fn prepare_managed(
        name: String,
        password: &[u8],
        author: &AuthorKey,
        identity: Identity,
        now: i64,
        policy: DatabasePolicy,
    ) -> Result<ArchiveSeed, ServiceError> {
        Self::prepare_managed_form(
            crate::CreateDatabase {
                name,
                description: None,
                policy,
            },
            password,
            author,
            identity,
            now,
        )
    }
    /// Prepare all creation form values before any working file is published.
    pub fn prepare_managed_form(
        form: crate::CreateDatabase,
        password: &[u8],
        author: &AuthorKey,
        identity: Identity,
        now: i64,
    ) -> Result<ArchiveSeed, ServiceError> {
        Self::prepare_managed_form_in(form, password, author, identity, now, Default::default())
    }
    /// Prepare creation with an explicit ciphertext allocator (including isolated workers).
    pub fn prepare_managed_form_in(
        form: crate::CreateDatabase,
        password: &[u8],
        author: &AuthorKey,
        identity: Identity,
        now: i64,
        temporary: taypeer_storage::TemporaryStorage,
    ) -> Result<ArchiveSeed, ServiceError> {
        let policy = form.policy;
        if identity.device != author.device_id() {
            return Err(ServiceError::Unauthorized);
        }
        let mut document =
            Document::new_with_writer(form.name.clone(), now, *author.device_id().as_bytes())?;
        if form.description.is_some() {
            document.update_metadata(form.name, form.description)?;
        }
        let (header, key) = taypeer_storage::create_epoch(password, policy.kdf_target_ms())?;
        let mut metadata = Checkpoint {
            version: 1,
            policy,
            policy_salt: codec::random_salt()?,
            keys: BTreeMap::from([(0, Zeroizing::new(*key.secret_bytes()))]),
            proofs: BTreeMap::new(),
            administration: BTreeMap::new(),
            recovery: recovery::Provenance::default(),
            blobs: BTreeMap::new(),
            processed: BTreeSet::new(),
            discarded: BTreeSet::new(),
            discarded_sources: BTreeSet::new(),
        };
        let chain = ControlChain::genesis(
            document.database_id().clone(),
            identity,
            author,
            metadata.commitment()?,
            taypeer_core::SchemaDescriptor::current(),
        )?;
        for source in document.changes_since(&[])? {
            metadata.proofs.insert(
                source.metadata().hash.clone(),
                SourceProof::sign(&chain, author, source.bytes())?,
            );
        }
        let clear = Zeroizing::new(document.export());
        let checkpoint = codec::seal_payload(
            &chain,
            author,
            ObjectKind::Checkpoint,
            &header,
            &key,
            &metadata,
            &clear,
            temporary.clone(),
        )?;
        let baseline = codec::seal_payload(
            &chain,
            author,
            ObjectKind::Baseline,
            &header,
            &key,
            &metadata,
            &clear,
            temporary.clone(),
        )?;
        Ok(ArchiveSeed {
            controls: chain.records().to_vec(),
            checkpoint: checkpoint.descriptor().digest,
            baseline: baseline.descriptor().digest,
            objects: vec![checkpoint, baseline],
        })
    }
    /// Authenticated worker authority last accepted into this unlocked session.
    /// This reads the retained snapshot and performs no coordinator I/O.
    pub fn session_authority(&self, session: &SessionToken) -> Result<ControlChain, ServiceError> {
        Ok(self
            .checked(session)?
            .managed
            .as_ref()
            .ok_or(ServiceError::InvalidContext)?
            .snapshot
            .chain()
            .clone())
    }

    /// Shared attachment/KDF settings authenticated by the current manager's control.
    pub fn database_policy(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<DatabasePolicy>, ServiceError> {
        Ok(stamped(session, self.checked(session)?.policy()))
    }
    /// Whether this session may author new changes under the latest local authority.
    pub fn can_write(&self, session: &SessionToken) -> Result<bool, ServiceError> {
        let state = self.checked(session)?;
        Ok(!state.write_uncertain
            && state.pending_icon.is_none()
            && self.compatibility(session)?.value.write.is_supported()
            && state.managed.as_ref().is_none_or(|m| m.writer().is_ok()))
    }
}
fn latest_baseline(snapshot: &ArchiveSnapshot) -> Result<EncryptedObject, ServiceError> {
    let active = snapshot.object(snapshot.metadata().manifest.body.baseline)?;
    if active.envelope().control == snapshot.chain().head_hash()? {
        return Ok(active);
    }
    for object in snapshot
        .metadata()
        .manifest
        .body
        .objects
        .values()
        .filter(|o| o.kind == ObjectKind::Baseline)
    {
        let object = snapshot.object(object.digest)?;
        if object.envelope().control == snapshot.chain().head_hash()? {
            return Ok(object);
        }
    }
    Err(ServiceError::AwaitingData)
}
