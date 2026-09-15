//! Raw source access for the authenticated service boundary. No network policy lives here.

use crate::{Document, Error};
use automerge::{ActorId, Automerge, Change, ChangeHash, ReadDoc};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;
use zeroize::Zeroizing;

const ACTOR_PREFIX: &[u8; 4] = b"TAY4";

/// Parsed causal information; trust comes from a separately verified source signature.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SourceMetadata {
    /// Original Automerge hash, unchanged by transport repackaging.
    pub hash: String,
    /// Exact original causal dependencies.
    pub dependencies: Vec<String>,
    /// Device identity encoded in the actor, absent for unbound demo sources.
    pub author: Option<[u8; 32]>,
}
impl std::fmt::Debug for SourceMetadata {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SourceMetadata { <redacted> }")
    }
}

/// An immutable parsed original change. Raw bytes contain secrets and never cross UI IPC.
pub struct OriginalChange {
    change: Change,
    metadata: SourceMetadata,
}
impl std::fmt::Debug for OriginalChange {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("OriginalChange { <redacted> }")
    }
}
impl OriginalChange {
    /// Parse bounded source bytes without applying them or issuing admission.
    pub fn parse(bytes: &[u8]) -> Result<Self, Error> {
        if bytes.len() > 64 * 1024 * 1024 {
            return Err(Error::InvalidDocument);
        }
        let change = Change::from_bytes(bytes.to_vec()).map_err(|_| Error::InvalidDocument)?;
        Ok(Self::from_change(change))
    }
    fn from_change(change: Change) -> Self {
        let actor = change.actor_id().to_bytes();
        let author = if actor.len() == 52 && &actor[..4] == ACTOR_PREFIX {
            Some(actor[4..36].try_into().expect("checked actor width"))
        } else {
            None
        };
        let metadata = SourceMetadata {
            hash: change.hash().to_string(),
            dependencies: change.deps().iter().map(ToString::to_string).collect(),
            author,
        };
        Self { change, metadata }
    }
    /// Causal information to validate against the signed original source proof.
    pub fn metadata(&self) -> &SourceMetadata {
        &self.metadata
    }
    /// Exact original uncompressed Automerge encoding for signing and encrypted storage.
    pub fn bytes(&self) -> &[u8] {
        self.change.raw_bytes()
    }
}

impl Document {
    /// Select an already verified author for subsequent changes. This does not grant
    /// network admission; the service must authenticate the source and current control.
    pub fn set_writer(&mut self, writer: [u8; 32]) {
        self.writer = Some(writer);
    }
    /// Clear local author state when attaching a read-only portable copy.
    pub fn clear_writer(&mut self) {
        self.writer = None;
    }
    /// Original sources not included in the given known causal heads.
    pub fn changes_since(&self, heads: &[String]) -> Result<Vec<OriginalChange>, Error> {
        let hashes = parse_hashes(heads)?;
        if hashes
            .iter()
            .any(|h| self.doc.get_change_by_hash(h).is_none())
        {
            return Err(Error::InvalidContext);
        }
        Ok(self
            .doc
            .get_changes(&hashes)
            .into_iter()
            .map(OriginalChange::from_change)
            .collect())
    }
    /// Whether a source has already been incorporated, independent of its ciphertext ID.
    pub fn contains_source(&self, hash: &str) -> Result<bool, Error> {
        let hash = hash
            .parse::<ChangeHash>()
            .map_err(|_| Error::InvalidContext)?;
        Ok(self.doc.get_change_by_hash(&hash).is_some())
    }
    /// Apply sources only after the service has checked signatures, author admission,
    /// schema, full dependencies and binary availability. No history row is invented.
    pub fn apply_sources(&mut self, sources: Vec<OriginalChange>) -> Result<(), Error> {
        let available: BTreeSet<_> = sources.iter().map(|s| s.metadata.hash.as_str()).collect();
        for source in &sources {
            for dependency in &source.metadata.dependencies {
                if !available.contains(dependency.as_str()) && !self.contains_source(dependency)? {
                    return Err(Error::InvalidContext);
                }
            }
        }
        let mut candidate = self.clone();
        let changes: Vec<_> = sources.into_iter().map(|s| s.change).collect();
        candidate.doc.apply_changes(changes)?;
        candidate.validate_structure()?;
        if candidate.database_id != self.database_id {
            return Err(Error::InvalidContext);
        }
        // Check the document's identity rather than only the cached Rust field.
        let clear = Zeroizing::new(candidate.export());
        let verified = Document::load(&clear)?;
        if verified.database_id != self.database_id {
            return Err(Error::InvalidContext);
        }
        self.doc = candidate.doc;
        self.name = verified.name;
        Ok(())
    }
    pub(crate) fn prepare_write(&mut self) -> Result<(), Error> {
        prepare_actor(&mut self.doc, self.writer)
    }
}
fn parse_hashes(heads: &[String]) -> Result<Vec<ChangeHash>, Error> {
    heads
        .iter()
        .map(|h| h.parse().map_err(|_| Error::InvalidContext))
        .collect()
}
pub(crate) fn prepare_actor(
    document: &mut Automerge,
    author: Option<[u8; 32]>,
) -> Result<(), Error> {
    if let Some(author) = author {
        let mut suffix = [0; 16];
        OsRng
            .try_fill_bytes(&mut suffix)
            .map_err(|_| Error::Random)?;
        let mut bytes = ACTOR_PREFIX.to_vec();
        bytes.extend(author);
        bytes.extend(suffix);
        // A fresh actor for every transaction also prevents transaction_at from
        // introducing Automerge's own concurrency prefix over the author binding.
        document.set_actor(ActorId::from(bytes));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolated_drafts_keep_author_bindings_and_original_conflicting_sources() {
        let mut left = Document::new_with_writer("PUBLIC sources", 1, [1; 32]).unwrap();
        let group = left.create_group("PUBLIC group".into(), None, 2).unwrap();
        let mut draft = left.begin_create_entry(group.id).unwrap();
        draft.fields_mut().title = "PUBLIC entry".into();
        let entry = left.save_entry(draft, 3).unwrap();
        let base = left.heads();
        let mut right = left.fork();
        right.set_writer([2; 32]);
        let mut a = left.begin_edit_entry(&entry).unwrap();
        let mut b = right.begin_edit_entry(&entry).unwrap();
        a.fields_mut().password = Some("PUBLIC A".into());
        b.fields_mut().password = Some("PUBLIC B".into());
        left.save_entry(a, 4).unwrap();
        right.save_entry(b, 5).unwrap();
        for source in left.changes_since(&[]).unwrap() {
            assert_eq!(source.metadata().author, Some([1; 32]));
        }
        let sources = right.changes_since(&base).unwrap();
        assert_eq!(sources.len(), 1);
        assert_eq!(sources[0].metadata().author, Some([2; 32]));
        let bytes = Zeroizing::new(sources[0].bytes().to_vec());
        left.apply_sources(sources).unwrap();
        let heads = left.heads();
        left.apply_sources(vec![OriginalChange::parse(&bytes).unwrap()])
            .unwrap();
        assert_eq!(left.heads(), heads);
        assert_eq!(left.history(&entry).unwrap().len(), 3);
        assert!(left.entry(&entry).unwrap().fields.is_none());
    }

    #[test]
    fn incomplete_dependencies_and_foreign_genesis_leave_the_document_unchanged() {
        let mut document = Document::new_with_writer("PUBLIC first", 1, [3; 32]).unwrap();
        let original = document.export();
        let other = Document::new_with_writer("PUBLIC other", 1, [4; 32]).unwrap();
        assert!(
            document
                .apply_sources(other.changes_since(&[]).unwrap())
                .is_err()
        );
        assert_eq!(document.export(), original);
        let mut peer = document.fork();
        peer.set_writer([5; 32]);
        let group = peer
            .create_group("PUBLIC unseen parent".into(), None, 2)
            .unwrap();
        let intermediate = peer.heads();
        peer.create_group("PUBLIC child".into(), Some(group.id), 3)
            .unwrap();
        assert!(
            document
                .apply_sources(peer.changes_since(&intermediate).unwrap())
                .is_err()
        );
        assert_eq!(document.export(), original);
    }
}
