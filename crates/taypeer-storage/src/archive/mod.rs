//! Portable ciphertext composition, independent of unlocked document ownership.

mod read;
mod write;

use crate::{EncryptedObject, Error, ObjectReader};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    path::{Path, PathBuf},
    sync::Arc,
};
use taypeer_trust::{
    CipherObject, ControlChain, Digest, Invitation, InvitationStatus, Manifest, SignedControl,
    SignedManifest, TransportKey,
};

const MAGIC: &[u8; 8] = b"TAYPEER\0";
// Archive layout is independent of the signed logical schema it transports.
const FORMAT: [u8; 4] = [1, 0, 5, 0];
const PREFIX: usize = 20;
const MAX_METADATA: u64 = 16 * 1024 * 1024;
const MAX_ARCHIVE: u64 = 16 * 1024 * 1024 * 1024;

/// A durable invitation record containing no bearer secret.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvitationRecord {
    /// Manager-authenticated public invitation.
    pub invitation: Invitation,
    /// Local admission workflow state.
    pub status: InvitationStatus,
}
/// Authenticated remote inventory. It grants no authority to apply its checkpoint.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct OfferedState {
    /// Full verifiable chain; origin root must match the admitted database.
    pub controls: Vec<SignedControl>,
    /// Remote ciphertext composition, not the local apply receipt.
    pub manifest: SignedManifest,
}
impl OfferedState {
    /// Authenticate the inventory before fetching any of its objects.
    pub fn verify(&self, root: Digest) -> Result<ControlChain, Error> {
        let chain = ControlChain::validate(self.controls.clone(), root)?;
        self.manifest.verify(&chain)?;
        Ok(chain)
    }
}

/// Nonsecret portable metadata covered by the manifest's auxiliary commitment.
/// Received remote journals never overwrite local apply receipts or draft state.
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveJournal {
    /// Pending and consumed invitations needed after manager restart/handoff.
    pub invitations: BTreeMap<Digest, InvitationRecord>,
    /// Remote inventories awaiting complete delivery or unlocked evaluation.
    pub offers: BTreeMap<Digest, OfferedState>,
    /// Locally accepted ciphertext checkpoints retained across rotation.
    pub retained: BTreeSet<Digest>,
    /// Independently valid contradictory signed chains; never silently discarded.
    pub forks: Vec<Vec<SignedControl>>,
}

/// Exact public metadata of one signed archive generation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ArchiveMetadata {
    /// Latest known control chain, possibly ahead of the decrypted checkpoint.
    pub controls: Vec<SignedControl>,
    /// Composition authenticated by the local transport key.
    pub manifest: SignedManifest,
    /// Exact metadata committed by the manifest.
    pub journal: ArchiveJournal,
}
impl ArchiveMetadata {
    /// Verify signatures, root, bounded collections and the complete auxiliary binding.
    pub fn verify(&self, pinned: Digest) -> Result<ControlChain, Error> {
        if self.journal.invitations.len() > 10_000
            || self.journal.offers.len() > 1024
            || self.journal.forks.len() > 16
        {
            return Err(Error::TooLarge);
        }
        let chain = ControlChain::validate(self.controls.clone(), pinned)?;
        self.manifest.verify(&chain)?;
        if self.manifest.body.auxiliary != auxiliary(&self.controls, &self.journal)? {
            return Err(Error::Authentication);
        }
        for (id, offer) in &self.journal.offers {
            let offered_chain = offer.verify(pinned)?;
            if offer.manifest.id()? != *id {
                return Err(Error::InvalidFile);
            }
            // A retained offer can be an older state; incomparable controls belong in evidence.
            chain.reconcile(&offered_chain)?;
        }
        for records in &self.journal.forks {
            let other = ControlChain::validate(records.clone(), pinned)?;
            if chain.reconcile(&other) != Err(taypeer_trust::Error::Fork) {
                return Err(Error::InvalidFile);
            }
        }
        for (id, record) in &self.journal.invitations {
            if record.invitation.id()? != *id {
                return Err(Error::InvalidFile);
            }
            // Expiry is a use-time check; a portable journal must retain consumed/expired records.
            let sequence = chain.at(record.invitation.control)?.sequence;
            let issuing = ControlChain::validate(
                self.controls
                    .iter()
                    .take_while(|c| c.body.sequence <= sequence)
                    .cloned()
                    .collect(),
                pinned,
            )?;
            record
                .invitation
                .verify(&issuing, record.invitation.issued_at)?;
        }
        if self
            .journal
            .retained
            .iter()
            .any(|id| !self.manifest.body.objects.contains_key(id))
        {
            return Err(Error::InvalidFile);
        }
        Ok(chain)
    }
    /// Derive an inventory for network transfer, without importing its local trust assertions.
    pub fn offer(&self) -> OfferedState {
        OfferedState {
            controls: self.controls.clone(),
            manifest: self.manifest.clone(),
        }
    }
}
fn auxiliary(controls: &[SignedControl], journal: &ArchiveJournal) -> Result<Digest, Error> {
    Ok(Digest::object(
        b"taypeer/archive-auxiliary/1",
        &(controls, journal),
    )?)
}

/// A stable verified file generation. Open readers keep the old inode across replacement.
#[derive(Clone)]
pub struct ArchiveSnapshot {
    file: Arc<File>,
    metadata: ArchiveMetadata,
    chain: ControlChain,
    offsets: BTreeMap<Digest, u64>,
    fingerprint: Digest,
    length: u64,
}
impl ArchiveSnapshot {
    /// Nonsecret compatibility after outer encoding, signatures and bounds were verified.
    /// A supported receive mode still requires admission and a durable writer before ACK.
    pub fn compatibility(
        &self,
        capabilities: &taypeer_core::ClientCapabilities,
    ) -> taypeer_core::CompatibilityReport {
        capabilities.assess(&self.chain.head().schema)
    }
    /// Authenticated metadata; none of it is an unlocked document.
    pub fn metadata(&self) -> &ArchiveMetadata {
        &self.metadata
    }
    /// Latest verified public authority.
    pub fn chain(&self) -> &ControlChain {
        &self.chain
    }
    /// Fingerprint used to reject external replacements and local rollback.
    pub fn fingerprint(&self) -> Digest {
        self.fingerprint
    }
    /// Physical bytes of this portable generation.
    pub fn length(&self) -> u64 {
        self.length
    }
    /// Copy the exact verified ciphertext generation from its stable inode to an
    /// inherited private IPC spool. This never opens or decrypts an object.
    pub fn copy_ciphertext(&self, output: &mut impl std::io::Write) -> Result<(), Error> {
        crate::encrypted_object::copy_exact(
            &mut ObjectReader::new(Arc::clone(&self.file), 0, self.length),
            output,
            self.length,
        )
    }
    /// Independently positioned ciphertext reader for a known object.
    pub fn reader(&self, id: Digest) -> Result<ObjectReader, Error> {
        let offset = *self.offsets.get(&id).ok_or(Error::MissingBlob)?;
        let descriptor = self
            .metadata
            .manifest
            .body
            .objects
            .get(&id)
            .ok_or(Error::InvalidFile)?;
        Ok(ObjectReader::new(
            Arc::clone(&self.file),
            offset,
            descriptor.length,
        ))
    }
    /// Create an independently owned encrypted staging object for an unlocked worker.
    pub fn object(&self, id: Digest) -> Result<EncryptedObject, Error> {
        let descriptor = self
            .metadata
            .manifest
            .body
            .objects
            .get(&id)
            .ok_or(Error::MissingBlob)?;
        EncryptedObject::receive(self.reader(id)?, descriptor, &self.chain)
    }
    /// Begin a candidate retaining every current section until explicitly removed.
    pub fn candidate(&self) -> ArchiveCandidate {
        ArchiveCandidate {
            base: Some(self.clone()),
            staged: BTreeMap::new(),
            retained: self
                .metadata
                .manifest
                .body
                .objects
                .keys()
                .copied()
                .collect(),
        }
    }
}

/// One coordinator transaction, owning only ciphertext and stable source readers.
pub struct ArchiveCandidate {
    base: Option<ArchiveSnapshot>,
    staged: BTreeMap<Digest, EncryptedObject>,
    retained: BTreeSet<Digest>,
}
impl ArchiveCandidate {
    /// Start a new file from fully checked encrypted objects.
    pub fn new() -> Self {
        Self {
            base: None,
            staged: BTreeMap::new(),
            retained: BTreeSet::new(),
        }
    }
    /// Add a complete immutable object. Equal IDs must have equal exact bytes.
    pub fn insert(&mut self, object: EncryptedObject) -> Result<Digest, Error> {
        let id = object.descriptor().digest;
        if let Some(existing) = self.descriptors().get(&id) {
            if existing != object.descriptor() {
                return Err(Error::BlobMismatch);
            }
            // Hashes accelerate identity; compare bytes before accepting deduplication.
            let mut old = self.reader(id)?;
            let mut new = object.reader()?;
            let mut left = vec![0; 1024 * 1024];
            let mut right = vec![0; 1024 * 1024];
            use std::io::Read;
            loop {
                let n = old.read(&mut left)?;
                new.read_exact(&mut right[..n])?;
                if left[..n] != right[..n] {
                    return Err(Error::BlobMismatch);
                }
                if n == 0 {
                    break;
                }
            }
            return Ok(id);
        }
        self.retained.insert(id);
        self.staged.insert(id, object);
        Ok(id)
    }
    /// Remove a section only after the unlocked service has checked all retention roots.
    pub fn remove(&mut self, id: Digest) {
        self.retained.remove(&id);
        self.staged.remove(&id);
    }
    /// Current exact candidate composition.
    pub fn descriptors(&self) -> BTreeMap<Digest, CipherObject> {
        let mut objects = self
            .base
            .as_ref()
            .map_or_else(BTreeMap::new, |b| b.metadata.manifest.body.objects.clone());
        objects.extend(
            self.staged
                .iter()
                .map(|(id, o)| (*id, o.descriptor().clone())),
        );
        objects.retain(|id, _| self.retained.contains(id));
        objects
    }
    fn reader(&self, id: Digest) -> Result<ObjectReader, Error> {
        if !self.retained.contains(&id) {
            return Err(Error::MissingBlob);
        }
        if let Some(object) = self.staged.get(&id) {
            return object.reader();
        }
        self.base.as_ref().ok_or(Error::MissingBlob)?.reader(id)
    }
    /// Authenticate all candidate roles and exact lengths when preparing metadata.
    pub fn metadata(
        &self,
        chain: &ControlChain,
        key: &TransportKey,
        mut body: Manifest,
        journal: ArchiveJournal,
    ) -> Result<ArchiveMetadata, Error> {
        body.signer = chain
            .at(body.control)?
            .members
            .iter()
            .find_map(|(id, member)| (member.identity.transport == key.public()).then_some(*id))
            .ok_or(taypeer_trust::Error::Unauthorized)?;
        body.objects = self.descriptors();
        body.auxiliary = auxiliary(chain.records(), &journal)?;
        let metadata = ArchiveMetadata {
            controls: chain.records().to_vec(),
            manifest: SignedManifest::sign(body, chain, key)?,
            journal,
        };
        metadata.verify(chain.root()?)?;
        Ok(metadata)
    }
}
impl Default for ArchiveCandidate {
    fn default() -> Self {
        Self::new()
    }
}

/// A two-phase local marker. It is not a portable root of trust or an OS rollback proof.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Anchor {
    /// Last completely confirmed file fingerprint, absent only during first creation.
    pub accepted: Option<Digest>,
    /// Prepared replacement, valid for recovery after an interrupted commit.
    pub prepared: Option<Digest>,
}
/// Platform-protected marker for one registered working copy. No secret plaintext enters it.
pub trait AnchorStore: Send + Sync {
    /// Read the marker, distinguishing a missing registration from an I/O failure.
    fn load(&self) -> Result<Option<Anchor>, Error>;
    /// Durably update the marker or return an error; callers never assume a failed write succeeded.
    fn save(&self, anchor: &Anchor) -> Result<(), Error>;
}

/// Sole writer for one portable file, retained by a coordinator even while locked.
pub struct ArchiveStore {
    path: PathBuf,
    _lock: File,
    snapshot: ArchiveSnapshot,
    anchor: Option<Arc<dyn AnchorStore>>,
    uncertain: bool,
}
impl ArchiveStore {
    /// Current verified immutable generation.
    pub fn snapshot(&self) -> &ArchiveSnapshot {
        &self.snapshot
    }
    /// Local path, never serialized as portable authority.
    pub fn path(&self) -> &Path {
        &self.path
    }
}

#[cfg(test)]
mod tests;
