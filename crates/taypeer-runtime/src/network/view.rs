//! Content-free desktop projections and cancellation shared by network callers.
use super::*;
use taypeer_trust::{DeviceId, InvitationStatus};

/// Cooperative cancellation of a network request, independent of database locking.
#[derive(Clone)]
pub struct NetworkCancellation(tokio::sync::watch::Sender<bool>);
impl Default for NetworkCancellation {
    fn default() -> Self {
        Self(tokio::sync::watch::channel(false).0)
    }
}
impl NetworkCancellation {
    /// Stop waiting for the peer. Already committed ciphertext is retained.
    pub fn cancel(&self) {
        self.0.send_replace(true);
    }
    pub(super) fn check(&self) -> Result<(), RuntimeError> {
        if *self.0.borrow() {
            Err(RuntimeError::Closed)
        } else {
            Ok(())
        }
    }
    pub(super) async fn run<T>(
        &self,
        future: impl std::future::Future<Output = Result<T, RuntimeError>>,
    ) -> Result<T, RuntimeError> {
        self.check()?;
        let mut cancelled = self.0.subscribe();
        tokio::select! {
            biased;
            _ = cancelled.wait_for(|value| *value) => Err(RuntimeError::Closed),
            result = future => result,
        }
    }
}

/// Verified public membership and the latest durable exchange attempt.
#[derive(Clone)]
pub struct DeviceExchange {
    /// Stable author identity, not a claimed display name.
    pub id: DeviceId,
    /// This device belongs to the current profile.
    pub local: bool,
    /// This member is the database's manager.
    pub manager: bool,
    /// Latest outbound exchange; never a claim of remote application or presence.
    pub progress: Option<PeerProgress>,
}
/// Public exchange state available even when the database is locked.
#[derive(Clone)]
pub struct DatabaseExchange {
    /// Logical database identity.
    pub database: DatabaseId,
    /// Whether the local member is the manager; service authorization remains authoritative.
    pub managing: bool,
    /// Verified members, with independent author and transport identities.
    pub devices: Vec<DeviceExchange>,
    /// Durable invitation workflow states; no bearer codes.
    pub invitations: Vec<(Digest, InvitationStatus)>,
}
/// Snapshot for presentation. Reading it never acquires an author credential.
#[derive(Clone, Default)]
pub struct NetworkSnapshot {
    /// The endpoint is running; this does not assert that any peer is reachable.
    pub running: bool,
    /// Registered working copies only, not every recent file.
    pub databases: Vec<DatabaseExchange>,
}
impl RuntimeHost {
    /// Read the currently registered public membership and safe exchange results.
    pub fn network_snapshot(&self) -> Result<NetworkSnapshot, RuntimeError> {
        let running = self.network_node().is_ok();
        let progress = if running {
            self.network_progress()?
        } else {
            Vec::new()
        };
        let databases: Vec<_> = self
            .context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .values()
            .map(|copy| copy.database.clone())
            .collect();
        let mut result = NetworkSnapshot {
            running,
            databases: Vec::new(),
        };
        for database in databases {
            let snapshot = self
                .context
                .coordinator
                .snapshot(&database)
                .map_err(sync_error)?;
            let head = snapshot.chain().head();
            let devices: Vec<_> = head
                .members
                .values()
                .map(|member| DeviceExchange {
                    id: member.identity.device,
                    local: member.identity.transport == self.context.transport.public(),
                    manager: member.identity.device == head.manager,
                    progress: progress
                        .iter()
                        .find(|p| p.database == database && p.peer == member.identity.transport)
                        .cloned(),
                })
                .collect();
            result.databases.push(DatabaseExchange {
                database,
                managing: devices.iter().any(|device| device.local && device.manager),
                devices,
                invitations: snapshot
                    .metadata()
                    .journal
                    .invitations
                    .iter()
                    .map(|(id, record)| (*id, record.status.clone()))
                    .collect(),
            });
        }
        Ok(result)
    }
    /// Schedule an immediate automatic exchange without starting duplicate per-peer jobs.
    pub fn wake_network(&self) -> Result<(), RuntimeError> {
        self.network
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .as_ref()
            .ok_or(RuntimeError::Closed)?
            .wake
            .notify_one();
        Ok(())
    }
    /// Exchange with the known route of one verified member or all admitted peers.
    /// Returns per-peer failures independently, preserving successful durable receipts.
    pub fn exchange_database(
        &self,
        database: &DatabaseId,
        selected: Option<DeviceId>,
        cancellation: &NetworkCancellation,
    ) -> Result<Vec<PeerProgress>, RuntimeError> {
        let snapshot = self
            .context
            .coordinator
            .snapshot(database)
            .map_err(sync_error)?;
        let routes = self.context.coordinator.routes().map_err(sync_error)?;
        let node = self.network_node()?;
        let mut result = Vec::new();
        for member in snapshot.chain().head().members.values() {
            cancellation.check()?;
            let peer = member.identity.transport;
            if peer == self.context.transport.public()
                || selected.is_some_and(|id| id != member.identity.device)
            {
                continue;
            }
            let exchange =
                match routes.get(&peer) {
                    Some(address) => self.runtime.block_on(cancellation.run(async {
                        Ok(node.exchange(address.clone(), database.clone()).await)
                    }))?,
                    None => Err(taypeer_sync::Error::Transport),
                };
            let progress = PeerProgress {
                database: database.clone(),
                peer,
                result: exchange,
            };
            if let Ok(slot) = self.network.lock()
                && let Some(network) = slot.as_ref()
                && Arc::ptr_eq(&network.node, &node)
                && let Ok(mut updates) = network.progress.lock()
            {
                updates.insert((database.clone(), peer), progress.clone());
            }
            result.push(progress);
        }
        Ok(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn cancellation_interrupts_a_peer_that_never_replies() {
        let runtime = tokio::runtime::Runtime::new().unwrap();
        let cancellation = NetworkCancellation::default();
        let trigger = cancellation.clone();
        let result: Result<(), RuntimeError> = runtime.block_on(async {
            tokio::spawn(async move {
                trigger.cancel();
            });
            cancellation.run(std::future::pending()).await
        });
        assert!(matches!(result, Err(RuntimeError::Closed)));
    }
}
