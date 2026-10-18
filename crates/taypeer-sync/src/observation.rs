//! Live transport facts, separate from durable ciphertext receipts and document application.
use crate::Error;
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Mutex},
};
use taypeer_core::DatabaseId;
use taypeer_trust::PublicKey;

/// Current authenticated connection availability for one authorized database and peer.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum PeerAvailability {
    /// No authorized connection is currently observed; this does not assert offline status.
    #[default]
    Unknown,
    /// At least one authenticated connection currently has permission for this database.
    Connected,
}

/// Current or last attempt in this endpoint lifetime, independent of durable receipt counts.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum ExchangePhase {
    /// No attempt, or a requested attempt is waiting for its per-peer exchange gate.
    #[default]
    Waiting,
    /// Establishing an authenticated transport connection.
    Connecting,
    /// An authorized connection is exchanging inventory or ciphertext.
    Transferring,
    /// The attempt completed; this does not claim remote document application.
    Completed,
    /// The attempt returned a transport, validation or storage error.
    Failed,
    /// The owning attempt was cancelled before completion.
    Cancelled,
}

/// Content-free facts for a live endpoint. Historical success never implies connectivity.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct PeerObservation {
    /// Active authorized connection state.
    pub availability: PeerAvailability,
    /// Latest attempt phase.
    pub phase: ExchangePhase,
}

type Key = (DatabaseId, PublicKey);
#[derive(Default)]
struct Entry {
    since: u64,
    generation: u64,
    phase: ExchangePhase,
    connections: BTreeSet<u64>,
}
#[derive(Default)]
struct State {
    next: u64,
    closed: bool,
    peers: BTreeMap<Key, Entry>,
}
#[derive(Clone, Default)]
pub(crate) struct Observations(Arc<Mutex<State>>);

impl Observations {
    pub(crate) fn begin(&self, database: DatabaseId, peer: PublicKey) -> Result<Attempt, Error> {
        let mut state = self.0.lock().map_err(|_| Error::State)?;
        if state.closed {
            return Err(Error::State);
        }
        state.next = state.next.checked_add(1).ok_or(Error::State)?;
        let generation = state.next;
        let key = (database, peer);
        let entry = state.peers.entry(key.clone()).or_insert_with(|| Entry {
            since: generation,
            ..Default::default()
        });
        entry.generation = generation;
        entry.phase = ExchangePhase::Waiting;
        Ok(Attempt {
            observations: self.clone(),
            key,
            generation,
            finished: false,
        })
    }
    pub(crate) fn snapshot(&self) -> Result<BTreeMap<Key, PeerObservation>, Error> {
        let state = self.0.lock().map_err(|_| Error::State)?;
        Ok(state
            .peers
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    PeerObservation {
                        availability: if entry.connections.is_empty() {
                            PeerAvailability::Unknown
                        } else {
                            PeerAvailability::Connected
                        },
                        phase: entry.phase,
                    },
                )
            })
            .collect())
    }
    pub(crate) fn forget(&self, database: &DatabaseId) -> Result<(), Error> {
        self.0
            .lock()
            .map_err(|_| Error::State)?
            .peers
            .retain(|(db, _), _| db != database);
        Ok(())
    }
    pub(crate) fn close(&self) {
        if let Ok(mut state) = self.0.lock() {
            state.closed = true;
            state.peers.clear();
        }
    }
}

pub(crate) struct Attempt {
    observations: Observations,
    key: Key,
    generation: u64,
    finished: bool,
}
impl Attempt {
    pub(crate) fn phase(&self, phase: ExchangePhase) {
        if let Ok(mut state) = self.observations.0.lock()
            && let Some(entry) = state.peers.get_mut(&self.key)
            && entry.generation == self.generation
        {
            entry.phase = phase;
        }
    }
    /// Only call after a successful backend authorization on the authenticated connection.
    pub(crate) fn authorized(&self) -> ConnectionObservation {
        if let Ok(mut state) = self.observations.0.lock()
            && let Some(entry) = state.peers.get_mut(&self.key)
            && self.generation >= entry.since
        {
            entry.connections.insert(self.generation);
        }
        self.phase(ExchangePhase::Transferring);
        ConnectionObservation {
            observations: self.observations.clone(),
            key: self.key.clone(),
            generation: self.generation,
        }
    }
    pub(crate) fn finish(mut self, success: bool) {
        self.phase(if success {
            ExchangePhase::Completed
        } else {
            ExchangePhase::Failed
        });
        self.finished = true;
    }
}
impl Drop for Attempt {
    fn drop(&mut self) {
        if !self.finished {
            self.phase(ExchangePhase::Cancelled);
        }
    }
}
pub(crate) struct ConnectionObservation {
    observations: Observations,
    key: Key,
    generation: u64,
}
impl Drop for ConnectionObservation {
    fn drop(&mut self) {
        if let Ok(mut state) = self.observations.0.lock()
            && let Some(entry) = state.peers.get_mut(&self.key)
        {
            entry.connections.remove(&self.generation);
        }
    }
}

/// One inbound connection can authorize multiple selected databases independently.
pub(crate) struct InboundObservation {
    observations: Observations,
    peer: PublicKey,
    databases: BTreeMap<DatabaseId, (Attempt, ConnectionObservation)>,
    failed: BTreeSet<DatabaseId>,
}
impl InboundObservation {
    pub(crate) fn new(observations: Observations, peer: PublicKey) -> Self {
        Self {
            observations,
            peer,
            databases: BTreeMap::new(),
            failed: BTreeSet::new(),
        }
    }
    pub(crate) fn authorized(&mut self, database: DatabaseId) -> Result<(), Error> {
        if !self.databases.contains_key(&database) {
            let attempt = self.observations.begin(database.clone(), self.peer)?;
            let connection = attempt.authorized();
            self.databases.insert(database, (attempt, connection));
        }
        Ok(())
    }
    pub(crate) fn finish(self, success: bool) {
        for (database, (attempt, _connection)) in self.databases {
            attempt.finish(success && !self.failed.contains(&database));
        }
    }
    pub(crate) fn failed(&mut self, database: &DatabaseId) {
        if self.databases.contains_key(database) {
            self.failed.insert(database.clone());
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn concurrent_connections_and_late_attempts_do_not_invent_presence() {
        let observations = Observations::default();
        let database = DatabaseId::new("PUBLIC observation");
        let peer = PublicKey::from_bytes([1; 32]);
        let key = (database.clone(), peer);
        let first = observations.begin(database.clone(), peer).unwrap();
        first.phase(ExchangePhase::Connecting);
        assert_eq!(
            observations.snapshot().unwrap()[&key].availability,
            PeerAvailability::Unknown
        );
        let connection = first.authorized();
        let second = observations.begin(database.clone(), peer).unwrap();
        let other = second.authorized();
        first.finish(false);
        assert_eq!(
            observations.snapshot().unwrap()[&key].phase,
            ExchangePhase::Transferring
        );
        drop(connection);
        assert_eq!(
            observations.snapshot().unwrap()[&key].availability,
            PeerAvailability::Connected
        );
        second.finish(true);
        drop(other);
        assert_eq!(
            observations.snapshot().unwrap()[&key],
            PeerObservation {
                availability: PeerAvailability::Unknown,
                phase: ExchangePhase::Completed
            }
        );
        let cancelled = observations.begin(database.clone(), peer).unwrap();
        drop(cancelled);
        assert_eq!(
            observations.snapshot().unwrap()[&key].phase,
            ExchangePhase::Cancelled
        );
        let late = observations.begin(database.clone(), peer).unwrap();
        let old = late.authorized();
        observations.forget(&database).unwrap();
        let fresh = observations.begin(database, peer).unwrap();
        let stale_connection = late.authorized();
        late.finish(true);
        drop(old);
        drop(stale_connection);
        assert_eq!(
            observations.snapshot().unwrap()[&key],
            PeerObservation::default()
        );
        observations.close();
        drop(fresh);
        assert!(observations.snapshot().unwrap().is_empty());
    }

    #[test]
    fn one_connection_keeps_database_authorization_and_failures_separate() {
        let observations = Observations::default();
        let first = DatabaseId::new("PUBLIC first database");
        let second = DatabaseId::new("PUBLIC second database");
        let peer = PublicKey::from_bytes([2; 32]);
        let mut inbound = InboundObservation::new(observations.clone(), peer);
        inbound.authorized(first.clone()).unwrap();
        inbound.authorized(second.clone()).unwrap();
        inbound.failed(&first);
        inbound.finish(true);
        let snapshot = observations.snapshot().unwrap();
        assert_eq!(snapshot[&(first, peer)].phase, ExchangePhase::Failed);
        assert_eq!(
            snapshot[&(second.clone(), peer)].phase,
            ExchangePhase::Completed
        );
        assert_eq!(
            snapshot[&(second, peer)].availability,
            PeerAvailability::Unknown
        );
    }
}
