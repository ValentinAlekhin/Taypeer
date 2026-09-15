//! Explicit inspection never merges an unadmitted source into the accepted document.
use super::*;
use serde::{Deserialize, Serialize};
use taypeer_core::OperationId;

/// Original-source identity, with no entry values or protected contents.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct ReceivedSource {
    /// Stable Automerge identity, independent of ciphertext packaging.
    pub change: String,
    /// Verified original author.
    pub author: taypeer_trust::DeviceId,
    /// Exact causal dependencies, not a display-time ordering.
    pub dependencies: Vec<String>,
    /// Explicit local disposition; discarded sources cannot be automatically applied.
    pub discarded: bool,
}

struct SourceIndex {
    snapshot: ArchiveSnapshot,
    sources: BTreeMap<String, (OriginalChange, SourceProof)>,
}
impl ManagedState {
    fn source_index(&self, document: &Document) -> Result<SourceIndex, ServiceError> {
        let snapshot = self.port.snapshot()?;
        if snapshot.chain() != self.snapshot.chain() {
            return Err(ServiceError::ExpiredSession);
        }
        let metadata = &self.open.as_ref().ok_or(ServiceError::Locked)?.metadata;
        let mut sources = BTreeMap::new();
        let mut bytes = 0usize;
        for source in document.changes_since(&[])? {
            let proof = metadata
                .proofs
                .get(&source.metadata().hash)
                .ok_or(ServiceError::InvalidDocument)?;
            apply::account_source(&mut bytes, &source, proof)?;
            sources.insert(source.metadata().hash.clone(), (source, proof.clone()));
        }
        for id in metadata.packet_ids(&snapshot) {
            let object = metadata.object(&snapshot, id)?;
            let packet = match self.read_packet(&object, metadata) {
                Ok(packet) => packet,
                Err(ServiceError::Storage(StorageError::Io)) => return Err(StorageError::Io.into()),
                // Unreadable packets remain in the archive and in apply diagnostics.
                Err(_) => continue,
            };
            for (source, proof) in packet.sources {
                let hash = source.metadata().hash.clone();
                if let Some((known, prior)) = sources.get(&hash) {
                    if known.bytes() != source.bytes() || prior != &proof {
                        return Err(ServiceError::InvalidDocument);
                    }
                } else {
                    apply::account_source(&mut bytes, &source, &proof)?;
                    sources.insert(hash, (source, proof));
                }
                if sources.len() > 100_000 {
                    return Err(StorageError::TooLarge.into());
                }
            }
        }
        Ok(SourceIndex { snapshot, sources })
    }

    fn inspect_source(
        &self,
        document: &Document,
        change: &str,
    ) -> Result<(Document, Checkpoint, ArchiveSnapshot), ServiceError> {
        if self
            .open
            .as_ref()
            .ok_or(ServiceError::Locked)?
            .metadata
            .discarded_sources
            .contains(change)
        {
            return Err(ServiceError::NotFound);
        }
        let index = self.source_index(document)?;
        if document.contains_source(change)? {
            return Err(ServiceError::InvalidContext);
        }
        let mut needed = BTreeSet::new();
        let mut queue = vec![change.to_owned()];
        while let Some(hash) = queue.pop() {
            if !needed.insert(hash.clone()) {
                continue;
            }
            let (source, _) = index.sources.get(&hash).ok_or(ServiceError::AwaitingData)?;
            queue.extend(source.metadata().dependencies.iter().cloned());
        }
        let mut candidate = document.clone();
        let mut metadata = self
            .open
            .as_ref()
            .ok_or(ServiceError::Locked)?
            .metadata
            .clone();
        let mut sources = Vec::new();
        for hash in needed {
            let (source, proof) = index.sources.get(&hash).ok_or(ServiceError::AwaitingData)?;
            apply::merge_bindings(&mut metadata.blobs, &proof.blobs)?;
            if !candidate.contains_source(&hash)? {
                sources.push(OriginalChange::parse(source.bytes())?);
            }
        }
        candidate.apply_sources(sources)?;
        Ok((candidate.at_source(change)?, metadata, index.snapshot))
    }
}

impl DatabaseService {
    /// List authenticated but unaccepted original changes. No document content is revealed.
    pub fn received_sources(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Vec<ReceivedSource>>, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        let index = managed.source_index(state.document())?;
        let metadata = &managed.open.as_ref().ok_or(ServiceError::Locked)?.metadata;
        let mut result = Vec::new();
        for (hash, (source, proof)) in index.sources {
            if state.document().contains_source(&hash)? {
                continue;
            }
            result.push(ReceivedSource {
                discarded: metadata.discarded_sources.contains(&hash),
                change: hash,
                author: proof.author,
                dependencies: source.metadata().dependencies.clone(),
            });
        }
        Ok(stamped(session, result))
    }

    /// Inspect a source at its exact causal state. Passwords and protected attributes stay masked.
    pub fn inspect_received(
        &self,
        session: &SessionToken,
        change: &str,
    ) -> Result<SessionValue<Vec<EntryView>>, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        let (source, _, _) = managed.inspect_source(state.document(), change)?;
        Ok(stamped(
            session,
            source
                .entries()?
                .into_iter()
                .map(views::entry_view)
                .collect(),
        ))
    }

    /// Reveal a selected password only through an explicit source-inspection action.
    pub fn reveal_received(
        &self,
        session: &SessionToken,
        change: &str,
        entry: &EntryId,
    ) -> Result<SessionValue<SecretValue>, ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        let (source, _, _) = managed.inspect_source(state.document(), change)?;
        Ok(stamped(session, views::password(source.entry(entry)?)?))
    }

    /// Explicitly discard one original source, without accepting it or removing dependencies.
    /// The marker is durable and applies to every future ciphertext packaging of this hash.
    pub fn discard_received(
        &mut self,
        session: &SessionToken,
        change: &str,
    ) -> Result<(), ServiceError> {
        let state = self.checked_mut(session)?;
        let document = state.document().clone();
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let mut metadata = managed
            .open
            .as_ref()
            .ok_or(ServiceError::Locked)?
            .metadata
            .clone();
        if metadata.discarded_sources.contains(change) {
            return Ok(());
        }
        if document.contains_source(change)? {
            return Err(ServiceError::InvalidContext);
        }
        let index = managed.source_index(&document)?;
        if !index.sources.contains_key(change) {
            return Err(ServiceError::NotFound);
        }
        managed.snapshot = index.snapshot;
        metadata.discarded_sources.insert(change.to_owned());
        managed.persist(&document, metadata, Vec::new(), None)
    }

    /// Extract an unambiguous selected entry as one new current-author confirmation.
    /// Source history is not admitted; the source remains available for further selections.
    pub fn extract_received(
        &mut self,
        session: &SessionToken,
        change: &str,
        entry: &EntryId,
        group: GroupId,
        operation: &OperationId,
    ) -> Result<SessionValue<EntryId>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let mut document = state.document().clone();
        if let Some(id) = document.extracted_entry(entry, change, &group, operation)? {
            return Ok(stamped(session, id));
        }
        let mut blobs = state.blobs()?.clone();
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let (source, metadata, snapshot) = managed.inspect_source(&document, change)?;
        let id = document.extract_entry(&source, (entry, change), group, operation, now)?;
        // Load exactly the result's required content, never all blobs in the foreign history.
        apply::load_blobs(&snapshot, &metadata, &document, &mut blobs)?;
        let attachments = document
            .entry(&id)?
            .fields
            .ok_or(ServiceError::InvalidDocument)?
            .attachments;
        if attachments.values().any(|attachment| {
            blobs
                .length(&attachment.blob)
                .is_none_or(|length| length > metadata.policy.attachment_bytes())
        }) || blobs.unique_bytes(&document.blob_references()?.attachments)
            > metadata.policy.total_attachment_bytes()
        {
            return Err(ServiceError::AttachmentLimit);
        }
        managed.snapshot = snapshot;
        let bindings = document
            .blob_references()?
            .required
            .into_iter()
            .map(|id| {
                let digest = *metadata.blobs.get(&id).ok_or(StorageError::MissingBlob)?;
                Ok((id, digest))
            })
            .collect::<Result<BTreeMap<_, _>, ServiceError>>()?;
        managed.commit_bound(&document, &blobs, &bindings)?;
        state.document = Some(document);
        state.blobs = Some(blobs);
        Ok(stamped(session, id))
    }
}
