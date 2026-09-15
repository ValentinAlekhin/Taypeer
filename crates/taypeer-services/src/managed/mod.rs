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
mod apply;
mod codec;
pub use apply::{ApplyReport, PendingPacket, PendingReason};
use codec::Checkpoint;

#[cfg(test)]
mod tests;

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
    open: Option<Unlocked>,
}
struct Unlocked {
    author: Option<AuthorKey>,
    header: Vec<u8>,
    metadata: Checkpoint,
    control: Digest,
}
impl ManagedState {
    pub fn policy(&self) -> DatabasePolicy {
        self.open
            .as_ref()
            .map_or_else(Default::default, |open| open.metadata.policy)
    }
    pub fn close(&mut self) {
        self.open = None;
    }
    pub fn check_edit_permission(&self) -> Result<(), ServiceError> {
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
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
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
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
        let author = self.writer()?;
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let chain = self.snapshot.chain();
        if open.control != chain.head_hash()? {
            return Err(ServiceError::AwaitingData);
        }
        let key = open.metadata.key(chain.head().epoch)?;
        let mut metadata = open.metadata.clone();
        let mut objects = self.seal_new_blobs(blobs, &mut metadata)?;
        for source in document.changes_since(&[])? {
            if let Some(proof) = metadata.proofs.get(&source.metadata().hash) {
                codec::verify_source(&source, proof, chain)?;
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
        mut metadata: Checkpoint,
        mut objects: Vec<EncryptedObject>,
        administration: Option<administration::Transition>,
    ) -> Result<(), ServiceError> {
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
        let key = metadata.key(chain.head().epoch)?;
        metadata.processed.insert(self.accepted);
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
        )?;
        let checkpoint_id = checkpoint.descriptor().digest;
        objects.push(checkpoint);
        let baseline = if administration.is_some() {
            let object = codec::seal_payload(
                chain,
                author,
                ObjectKind::Baseline,
                header,
                &key,
                &metadata,
                &clear,
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
        let request = PreparedCommit {
            expected: self.snapshot.fingerprint(),
            control: self.snapshot.chain().head_hash()?,
            controls: chain.records().to_vec(),
            objects,
            remove: BTreeSet::new(),
            checkpoint: checkpoint_id,
            baseline,
            journal,
        };
        let snapshot = self.port.commit(request)?;
        let open = self.open.as_mut().ok_or(ServiceError::Locked)?;
        if let Some(transition) = administration {
            open.header = transition.header;
        }
        open.metadata = metadata;
        open.control = snapshot.chain().head_hash()?;
        self.baseline = baseline;
        self.accepted = checkpoint_id;
        self.snapshot = snapshot;
        Ok(())
    }
    pub fn save_draft(&self, draft: &DraftState, blobs: &BlobStore) -> Result<(), ServiceError> {
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
    fn load_draft(&self, blobs: &mut BlobStore) -> Result<Option<DraftState>, ServiceError> {
        let Some(object) = self.port.load_draft(self.snapshot.chain())? else {
            return Ok(None);
        };
        let open = self.open.as_ref().ok_or(ServiceError::Locked)?;
        let (clear, staged) = object.unlock_bundle(&open.metadata.key(object.envelope().epoch)?)?;
        let (working_copy, draft): (Digest, DraftState) =
            serde_json::from_slice(&clear).map_err(|_| ServiceError::InvalidDocument)?;
        if working_copy != self.port.working_copy()
            || draft
                .binary_references()
                .iter()
                .any(|id| staged.length(id).is_none())
        {
            return Err(ServiceError::InvalidContext);
        }
        blobs.import(&staged)?;
        Ok(Some(draft))
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
        if identity.device != author.device_id() {
            return Err(ServiceError::Unauthorized);
        }
        let document = Document::new_with_writer(name, now, *author.device_id().as_bytes())?;
        let (header, key) = taypeer_storage::create_epoch(password, policy.kdf_target_ms())?;
        let mut metadata = Checkpoint {
            version: 1,
            policy,
            policy_salt: codec::random_salt()?,
            keys: BTreeMap::from([(0, Zeroizing::new(*key.secret_bytes()))]),
            proofs: BTreeMap::new(),
            administration: BTreeMap::new(),
            blobs: BTreeMap::new(),
            processed: BTreeSet::new(),
            discarded: BTreeSet::new(),
        };
        let chain = ControlChain::genesis(
            document.database_id().clone(),
            identity,
            author,
            metadata.commitment()?,
            4,
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
        )?;
        let baseline = codec::seal_payload(
            &chain,
            author,
            ObjectKind::Baseline,
            &header,
            &key,
            &metadata,
            &clear,
        )?;
        Ok(ArchiveSeed {
            controls: chain.records().to_vec(),
            checkpoint: checkpoint.descriptor().digest,
            baseline: baseline.descriptor().digest,
            objects: vec![checkpoint, baseline],
        })
    }
    /// Authenticate a registered or copied archive. The credential callback runs only
    /// after successful password authentication. An unadmitted identity gets read/export access.
    pub fn open_managed(
        &mut self,
        port: Box<dyn CipherPersistence>,
        password: &[u8],
        author: impl FnOnce() -> Result<Option<AuthorKey>, ServiceError>,
    ) -> Result<SessionToken, ServiceError> {
        let snapshot = port.snapshot()?;
        let database = snapshot.chain().head().database.clone();
        let generation = match self.databases.get(&database) {
            None => 1,
            Some(prior)
                if !prior.unlocked
                    && prior.managed.as_ref().is_some_and(|managed| {
                        managed.port.path() == port.path()
                            && managed.port.working_copy() == port.working_copy()
                    }) =>
            {
                prior
                    .generation
                    .checked_add(1)
                    .ok_or(ServiceError::InvalidContext)?
            }
            Some(_) => return Err(ServiceError::InvalidContext),
        };
        let active = snapshot.object(snapshot.metadata().manifest.body.checkpoint)?;
        let latest = snapshot.chain().head_hash()?;
        // A learned rotation cannot be opened using the superseded password wrapper.
        let mut selected = active.clone();
        if active.envelope().epoch != snapshot.chain().head().epoch
            || active.envelope().control != latest
        {
            selected = latest_baseline(&snapshot)?;
        }
        let key = selected.unlock_key(password)?;
        let (mut metadata, mut document) =
            codec::read_checkpoint(&selected, &key, snapshot.chain())?;
        let author = author()?;
        let admitted = author
            .as_ref()
            .is_some_and(|a| snapshot.chain().head().members.contains_key(&a.device_id()));
        if admitted
            && selected.envelope().kind != ObjectKind::Baseline
            && author
                .as_ref()
                .is_some_and(|a| a.device_id() != selected.envelope().author)
        {
            selected = latest_baseline(&snapshot)?;
            (metadata, document) = codec::read_checkpoint(
                &selected,
                &metadata.key(selected.envelope().epoch)?,
                snapshot.chain(),
            )?;
        }
        let own_checkpoint = author
            .as_ref()
            .is_some_and(|a| a.device_id() == active.envelope().author);
        if selected.descriptor().digest != active.descriptor().digest
            && (own_checkpoint || !admitted)
        {
            let (prior, accepted) = codec::read_checkpoint(
                &active,
                &metadata.key(active.envelope().epoch)?,
                snapshot.chain(),
            )?;
            document.merge(&accepted)?;
            metadata = apply::merge_accepted_metadata(metadata, &prior)?;
        }
        if !own_checkpoint {
            // Portable receipt claims from another device are never local apply/discard decisions.
            metadata.processed.clear();
            metadata.discarded.clear();
        }
        if let Some(author) = author.as_ref().filter(|_| admitted) {
            document.set_writer(*author.device_id().as_bytes());
        } else {
            document.clear_writer();
        }
        let baseline = if selected.envelope().kind == ObjectKind::Baseline {
            selected.descriptor().digest
        } else {
            snapshot.metadata().manifest.body.baseline
        };
        let managed = ManagedState {
            port,
            baseline,
            accepted: selected.descriptor().digest,
            snapshot,
            open: Some(Unlocked {
                author,
                header: selected.password_header()?,
                metadata,
                control: selected.envelope().control,
            }),
        };
        let mut blobs = BlobStore::new()?;
        managed.load_required_blobs(&document, &mut blobs)?;
        let draft = managed.load_draft(&mut blobs)?;
        let label = managed
            .port
            .path()
            .file_name()
            .ok_or(ServiceError::InvalidContext)?
            .to_string_lossy()
            .into_owned();
        self.databases.insert(
            database.clone(),
            DatabaseState {
                document: Some(document),
                blobs: Some(blobs),
                label,
                file: None,
                managed: Some(managed),
                key: None,
                generation,
                unlocked: true,
                draft,
            },
        );
        Ok(SessionToken {
            database,
            generation,
        })
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
        Ok(state.managed.as_ref().is_none_or(|m| m.writer().is_ok()))
    }
}
fn latest_baseline(snapshot: &ArchiveSnapshot) -> Result<EncryptedObject, ServiceError> {
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
