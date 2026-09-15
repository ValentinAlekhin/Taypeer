//! The single writer of registered ciphertext copies. No decrypted state is owned here.

use crate::{Backend, Command, EndpointAddr, Error, Reply};
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    sync::{Arc, Mutex},
};
use taypeer_core::DatabaseId;
use taypeer_storage::{
    ArchiveMetadata, ArchiveSnapshot, ArchiveStore, EncryptedObject, ObjectReader, PreparedCommit,
};
use taypeer_trust::{
    CipherObject, ControlChain, Digest, InvitationSecret, InvitationStatus, PublicKey, TransportKey,
};
use tempfile::NamedTempFile;
use tokio::sync::broadcast;

/// Events are invalidation/progress signals, never decrypted data or successful application claims.
#[derive(Clone, Debug)]
pub enum CoordinatorEvent {
    /// A complete object is durable and may be forwarded while locked.
    Received {
        /// Selected database.
        database: DatabaseId,
        /// Ciphertext identity.
        object: Digest,
    },
    /// Latest known authority changed; epoch/revocation changes require immediate worker closure.
    ControlChanged {
        /// Selected database.
        database: DatabaseId,
        /// Latest signed control.
        control: Digest,
        /// The existing unlocked process must be invalidated.
        lock: bool,
    },
    /// A contradictory signed chain stops management, exchange and application.
    Frozen(DatabaseId),
    /// A verified recipient is waiting for explicit manager approval.
    JoinRequested {
        /// Selected database.
        database: DatabaseId,
        /// Durable request identity.
        request: Digest,
    },
    /// A locally prepared candidate has been durably committed.
    Committed(DatabaseId),
}

struct WorkingCopy {
    store: ArchiveStore,
    frozen: bool,
}
/// Shared host-side coordinator for several independent databases under one profile.
/// It serializes blocking disk transactions; network I/O runs outside this mutex.
pub struct Coordinator {
    copies: Mutex<BTreeMap<DatabaseId, WorkingCopy>>,
    key: Arc<TransportKey>,
    routes: Mutex<BTreeMap<PublicKey, EndpointAddr>>,
    events: broadcast::Sender<CoordinatorEvent>,
}
impl Coordinator {
    pub(super) fn check_path(
        &self,
        database: &DatabaseId,
        path: &std::path::Path,
    ) -> Result<(), Error> {
        let copies = self.copies.lock().map_err(|_| Error::State)?;
        if copies.get(database).ok_or(Error::State)?.store.path() != path {
            return Err(Error::State);
        }
        Ok(())
    }
    /// Construct without opening any database or revealing an author credential.
    pub fn new(key: Arc<TransportKey>) -> Self {
        let (events, _) = broadcast::channel(256);
        Self {
            copies: Mutex::new(BTreeMap::new()),
            key,
            routes: Mutex::new(BTreeMap::new()),
            events,
        }
    }
    /// Register an already verified, exclusively locked local file.
    pub fn register(&self, store: ArchiveStore) -> Result<(), Error> {
        let id = store.snapshot().chain().head().database.clone();
        let frozen = !store.snapshot().metadata().journal.forks.is_empty();
        let mut copies = self.copies.lock().map_err(|_| Error::State)?;
        if copies.contains_key(&id) {
            return Err(Error::State);
        }
        copies.insert(id, WorkingCopy { store, frozen });
        Ok(())
    }
    /// Stop using a working file and release its one-writer lock.
    pub fn unregister(&self, database: &DatabaseId) -> Result<(), Error> {
        self.copies
            .lock()
            .map_err(|_| Error::State)?
            .remove(database)
            .ok_or(Error::State)?;
        Ok(())
    }
    /// Obtain a stable ciphertext snapshot for an unlocked worker or explicit export.
    /// Reading remains possible when signed management has forked.
    pub fn snapshot(&self, database: &DatabaseId) -> Result<ArchiveSnapshot, Error> {
        let copies = self.copies.lock().map_err(|_| Error::State)?;
        Ok(copies
            .get(database)
            .ok_or(Error::State)?
            .store
            .snapshot()
            .clone())
    }
    /// Subscribe to invalidation/progress. A lagged subscriber must re-read all authority states.
    pub fn subscribe(&self) -> broadcast::Receiver<CoordinatorEvent> {
        self.events.subscribe()
    }
    /// Current authenticated peer routes, learned only through admitted exchanges.
    pub fn routes(&self) -> Result<BTreeMap<PublicKey, EndpointAddr>, Error> {
        Ok(self.routes.lock().map_err(|_| Error::State)?.clone())
    }
    /// Store an explicitly received invitation route, bound to its authenticated identity.
    pub fn remember_route(&self, address: EndpointAddr) -> Result<(), Error> {
        self.routes
            .lock()
            .map_err(|_| Error::State)?
            .insert(PublicKey::from_bytes(*address.id.as_bytes()), address);
        Ok(())
    }
    /// Commit an unlocked candidate after rechecking authority and the exact local generation.
    /// Incoming objects cannot be overwritten by a candidate prepared before their receipt.
    pub fn commit(
        &self,
        database: &DatabaseId,
        request: PreparedCommit,
    ) -> Result<ArchiveSnapshot, Error> {
        let mut copies = self.copies.lock().map_err(|_| Error::State)?;
        let copy = copies.get_mut(database).ok_or(Error::State)?;
        if copy.frozen {
            return Err(taypeer_trust::Error::Fork.into());
        }
        let snapshot = copy.store.snapshot().clone();
        if snapshot.fingerprint() != request.expected
            || snapshot.chain().head_hash()? != request.control
        {
            return Err(taypeer_storage::Error::Changed.into());
        }
        let chain = ControlChain::validate(request.controls, snapshot.chain().root()?)?;
        if snapshot.chain().reconcile(&chain)? != chain {
            return Err(taypeer_trust::Error::Stale.into());
        }
        let mut candidate = snapshot.candidate();
        for object in request.objects {
            candidate.insert(object)?;
        }
        for id in request.remove {
            candidate.remove(id);
        }
        let mut body = snapshot.metadata().manifest.body.clone();
        body.control = chain.head_hash()?;
        body.generation = body.generation.checked_add(1).ok_or(Error::State)?;
        body.checkpoint = request.checkpoint;
        body.baseline = request.baseline;
        let metadata = candidate.metadata(&chain, &self.key, body, request.journal)?;
        copy.store.commit(request.expected, &candidate, metadata)?;
        let result = copy.store.snapshot().clone();
        // No listeners is normal, e.g. during a headless one-shot command.
        let _ = self
            .events
            .send(CoordinatorEvent::Committed(database.clone()));
        self.notify_control(database, &snapshot, &result)?;
        Ok(result)
    }
    /// Preserve a separately named source before a manager rotation/recovery operation.
    pub fn preserve_before(&self, database: &DatabaseId, operation: Digest) -> Result<(), Error> {
        self.copies
            .lock()
            .map_err(|_| Error::State)?
            .get(database)
            .ok_or(Error::State)?
            .store
            .preserve_before(operation)?;
        Ok(())
    }
    fn notify_control(
        &self,
        database: &DatabaseId,
        old: &ArchiveSnapshot,
        new: &ArchiveSnapshot,
    ) -> Result<(), Error> {
        if old.chain().head_hash()? != new.chain().head_hash()? {
            let lock = old.chain().head().epoch != new.chain().head().epoch
                || new.chain().admit_transport(self.key.public()).is_err();
            let _ = self.events.send(CoordinatorEvent::ControlChanged {
                database: database.clone(),
                control: new.chain().head_hash()?,
                lock,
            });
        }
        Ok(())
    }
    fn offer(&self, peer: PublicKey, incoming: ArchiveMetadata) -> Result<Reply, Error> {
        let database = incoming.manifest.body.database.clone();
        let mut copies = self.copies.lock().map_err(|_| Error::State)?;
        let copy = copies.get_mut(&database).ok_or(Error::Unauthorized)?;
        authorize(copy, peer)?;
        let snapshot = copy.store.snapshot().clone();
        let incoming_chain = incoming.verify(snapshot.chain().root()?)?;
        if !incoming.journal.forks.is_empty() {
            copy.frozen = true;
            let _ = self.events.send(CoordinatorEvent::Frozen(database.clone()));
            let mut journal = snapshot.metadata().journal.clone();
            // Evidence can fork a descendant not yet known locally. Preserve that
            // descendant as our comparable chain before recording its contradiction.
            let common = match snapshot.chain().reconcile(&incoming_chain) {
                Ok(chain) => chain,
                Err(taypeer_trust::Error::Fork) => {
                    if !journal.forks.contains(&incoming.controls) {
                        journal.forks.push(incoming.controls.clone());
                    }
                    snapshot.chain().clone()
                }
                Err(error) => return Err(error.into()),
            };
            for records in incoming.journal.forks {
                let evidence = ControlChain::validate(records.clone(), common.root()?)?;
                if common.reconcile(&evidence) == Err(taypeer_trust::Error::Fork)
                    && !journal.forks.contains(&records)
                {
                    journal.forks.push(records);
                }
            }
            let candidate = snapshot.candidate();
            let mut body = snapshot.metadata().manifest.body.clone();
            body.generation = body.generation.checked_add(1).ok_or(Error::State)?;
            let metadata = candidate.metadata(&common, &self.key, body, journal)?;
            copy.store
                .commit(snapshot.fingerprint(), &candidate, metadata)?;
            return Err(taypeer_trust::Error::Fork.into());
        }
        let chain = match snapshot.chain().reconcile(&incoming_chain) {
            Ok(chain) => chain,
            Err(taypeer_trust::Error::Fork) => {
                copy.frozen = true;
                let _ = self.events.send(CoordinatorEvent::Frozen(database));
                let mut journal = snapshot.metadata().journal.clone();
                if !journal.forks.contains(&incoming.controls) {
                    journal.forks.push(incoming.controls);
                }
                let candidate = snapshot.candidate();
                let mut body = snapshot.metadata().manifest.body.clone();
                body.generation = body.generation.checked_add(1).ok_or(Error::State)?;
                let metadata = candidate.metadata(snapshot.chain(), &self.key, body, journal)?;
                copy.store
                    .commit(snapshot.fingerprint(), &candidate, metadata)?;
                return Err(taypeer_trust::Error::Fork.into());
            }
            Err(error) => return Err(error.into()),
        };
        let need: BTreeSet<_> = incoming
            .manifest
            .body
            .objects
            .keys()
            .filter(|id| !snapshot.metadata().manifest.body.objects.contains_key(id))
            .copied()
            .collect();
        // Transport manifests also describe local receipt journals. Echoing every new
        // manifest ID would create an endless exchange even when all data is present.
        if need.is_empty() && chain == *snapshot.chain() && incoming.journal.forks.is_empty() {
            return Ok(Reply::Needed(need));
        }
        let offer = incoming.offer();
        let id = offer.manifest.id()?;
        let mut journal = snapshot.metadata().journal.clone();
        let changed = journal.offers.get(&id) != Some(&offer) || chain != *snapshot.chain();
        journal.offers.insert(id, offer);
        if changed {
            let candidate = snapshot.candidate();
            let mut body = snapshot.metadata().manifest.body.clone();
            body.generation = body.generation.checked_add(1).ok_or(Error::State)?;
            // Keep the old decrypted checkpoint's control while learning newer authority.
            let metadata = candidate.metadata(&chain, &self.key, body, journal)?;
            copy.store
                .commit(snapshot.fingerprint(), &candidate, metadata)?;
            self.notify_control(
                &snapshot.chain().head().database,
                &snapshot,
                copy.store.snapshot(),
            )?;
        }
        Ok(Reply::Needed(need))
    }
}
fn authorize(copy: &WorkingCopy, peer: PublicKey) -> Result<(), Error> {
    if copy.frozen {
        return Err(taypeer_trust::Error::Fork.into());
    }
    copy.store.snapshot().chain().admit_transport(peer)?;
    Ok(())
}
fn announced(snapshot: &ArchiveSnapshot, descriptor: &CipherObject) -> bool {
    snapshot
        .metadata()
        .manifest
        .body
        .objects
        .get(&descriptor.digest)
        == Some(descriptor)
        || snapshot
            .metadata()
            .journal
            .offers
            .values()
            .any(|offer| offer.manifest.body.objects.get(&descriptor.digest) == Some(descriptor))
}
impl Backend for Coordinator {
    fn command(&self, peer: PublicKey, command: Command) -> Result<Reply, Error> {
        match command {
            Command::Inventory { database, address } => {
                if PublicKey::from_bytes(*address.id.as_bytes()) != peer {
                    return Err(Error::Unauthorized);
                }
                let copies = self.copies.lock().map_err(|_| Error::State)?;
                let copy = copies.get(&database).ok_or(Error::Unauthorized)?;
                authorize(copy, peer)?;
                self.remember_route(address)?;
                Ok(Reply::Inventory(Box::new(
                    copy.store.snapshot().metadata().clone(),
                )))
            }
            Command::Offer(metadata) => self.offer(peer, *metadata),
            Command::Join {
                invitation,
                secret,
                proof,
                address,
            } => {
                if PublicKey::from_bytes(*address.id.as_bytes()) != peer {
                    return Err(Error::Unauthorized);
                }
                let database = invitation.database.clone();
                let mut copies = self.copies.lock().map_err(|_| Error::State)?;
                let copy = copies.get_mut(&database).ok_or(Error::Unauthorized)?;
                if copy.frozen {
                    return Err(taypeer_trust::Error::Fork.into());
                }
                let snapshot = copy.store.snapshot().clone();
                if snapshot.chain().head().members[&snapshot.chain().head().manager]
                    .identity
                    .transport
                    != self.key.public()
                {
                    return Err(Error::Unauthorized);
                }
                let id = invitation.id()?;
                let mut journal = snapshot.metadata().journal.clone();
                let record = journal
                    .invitations
                    .get_mut(&id)
                    .ok_or(Error::Unauthorized)?;
                if record.invitation != *invitation {
                    return Err(Error::Unauthorized);
                }
                let token = InvitationSecret::from_bytes(*secret);
                record.status.request(
                    &invitation,
                    &token,
                    *proof,
                    peer,
                    snapshot.chain(),
                    unix_seconds()?,
                )?;
                if journal != snapshot.metadata().journal {
                    let candidate = snapshot.candidate();
                    let mut body = snapshot.metadata().manifest.body.clone();
                    body.generation = body.generation.checked_add(1).ok_or(Error::State)?;
                    let metadata =
                        candidate.metadata(snapshot.chain(), &self.key, body, journal)?;
                    copy.store
                        .commit(snapshot.fingerprint(), &candidate, metadata)?;
                }
                self.remember_route(address)?;
                let _ = self.events.send(CoordinatorEvent::JoinRequested {
                    database,
                    request: id,
                });
                Ok(Reply::JoinPending(id))
            }
            Command::JoinStatus { database, request } => {
                let copies = self.copies.lock().map_err(|_| Error::State)?;
                let copy = copies.get(&database).ok_or(Error::Unauthorized)?;
                if copy.frozen {
                    return Err(taypeer_trust::Error::Fork.into());
                }
                let snapshot = copy.store.snapshot();
                let record = snapshot
                    .metadata()
                    .journal
                    .invitations
                    .get(&request)
                    .ok_or(Error::Unauthorized)?;
                match &record.status {
                    InvitationStatus::Requested(proof) if proof.recipient.transport == peer => {
                        if record
                            .invitation
                            .verify(snapshot.chain(), unix_seconds()?)
                            .is_err()
                        {
                            return Ok(Reply::JoinRejected);
                        }
                        Ok(Reply::JoinPending(request))
                    }
                    InvitationStatus::Accepted { recipient, control } => {
                        let admitted = snapshot
                            .chain()
                            .at(*control)?
                            .members
                            .get(recipient)
                            .ok_or(Error::Unauthorized)?;
                        if admitted.identity.transport != peer {
                            return Err(Error::Unauthorized);
                        }
                        authorize(copy, peer)?;
                        Ok(Reply::Joined(Box::new(snapshot.metadata().clone())))
                    }
                    InvitationStatus::Rejected | InvitationStatus::Cancelled => {
                        Ok(Reply::JoinRejected)
                    }
                    _ => Err(Error::Unauthorized),
                }
            }
        }
    }
    fn authorize_object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: &CipherObject,
    ) -> Result<(), Error> {
        let copies = self.copies.lock().map_err(|_| Error::State)?;
        let copy = copies.get(database).ok_or(Error::Unauthorized)?;
        authorize(copy, peer)?;
        if !announced(copy.store.snapshot(), descriptor) {
            return Err(Error::Unauthorized);
        }
        Ok(())
    }
    fn object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        id: Digest,
    ) -> Result<(CipherObject, ObjectReader), Error> {
        let copies = self.copies.lock().map_err(|_| Error::State)?;
        let copy = copies.get(database).ok_or(Error::Unauthorized)?;
        authorize(copy, peer)?;
        let snapshot = copy.store.snapshot();
        let descriptor = snapshot
            .metadata()
            .manifest
            .body
            .objects
            .get(&id)
            .ok_or(Error::State)?
            .clone();
        Ok((descriptor, snapshot.reader(id)?))
    }
    fn receive(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: CipherObject,
        file: NamedTempFile,
    ) -> Result<Digest, Error> {
        let mut copies = self.copies.lock().map_err(|_| Error::State)?;
        let copy = copies.get_mut(database).ok_or(Error::Unauthorized)?;
        authorize(copy, peer)?;
        let snapshot = copy.store.snapshot().clone();
        if !announced(&snapshot, &descriptor) {
            return Err(Error::Unauthorized);
        }
        let object = EncryptedObject::receive(
            File::open(file.path()).map_err(|_| Error::Storage(taypeer_storage::Error::Io))?,
            &descriptor,
            snapshot.chain(),
        )?;
        if !snapshot
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&descriptor.digest)
        {
            let mut candidate = snapshot.candidate();
            candidate.insert(object)?;
            let mut body = snapshot.metadata().manifest.body.clone();
            body.generation = body.generation.checked_add(1).ok_or(Error::State)?;
            let metadata = candidate.metadata(
                snapshot.chain(),
                &self.key,
                body,
                snapshot.metadata().journal.clone(),
            )?;
            copy.store
                .commit(snapshot.fingerprint(), &candidate, metadata)?;
        }
        let _ = self.events.send(CoordinatorEvent::Received {
            database: database.clone(),
            object: descriptor.digest,
        });
        Ok(descriptor.digest)
    }
}
fn unix_seconds() -> Result<u64, Error> {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|t| t.as_secs())
        .map_err(|_| Error::State)
}
