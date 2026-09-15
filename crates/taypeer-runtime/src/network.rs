//! Explicit network lifetime, authenticated routes and resumable invitation downloads.
use crate::{
    RuntimeError, RuntimeHost, Worker,
    cipher_ipc::storage,
    host::{canonical_path, sync_error},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    fs::File,
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
    time::Duration,
};
use taypeer_core::DatabaseId;
use taypeer_storage::{ArchiveCandidate, ArchiveMetadata, ArchiveStore, EncryptedObject};
use taypeer_sync::{Command, EndpointAddr, ExchangeReport, Node, RelaySetting, Reply};
use taypeer_trust::{Digest, Invitation, JoinProof, PublicKey};
use zeroize::Zeroizing;

/// Explicitly revealed invitation code. Never Debug/log, argv, history or ordinary status output.
#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InvitationCode {
    /// Manager-signed public invitation.
    pub invitation: Invitation,
    /// Five-minute random bearer secret; not retained in the profile.
    pub secret: Zeroizing<[u8; 32]>,
    /// Route bound to the invitation's transport identity.
    pub address: EndpointAddr,
}
/// Local resumable enrollment progress. All fields are public; the bearer is deliberately absent.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingJoin {
    /// Exact signed invitation and externally pinned root.
    pub invitation: Invitation,
    /// Recipient's author/transport proof.
    pub proof: JoinProof,
    /// Last known manager route.
    pub address: EndpointAddr,
    /// Explicit selected new working-copy destination.
    pub path: PathBuf,
}
/// Progress distinguishes a pending approval from a completely saved encrypted database.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub enum JoinProgress {
    /// Manager approval has not arrived; the request can be polled after restart.
    Pending(Digest),
    /// A complete file and its native registration are durable; it is still locked.
    Received(DatabaseId),
    /// Request was refused, cancelled or expired without approval.
    Rejected,
}
/// Content-free result of the most recent attempt to one admitted peer.
#[derive(Clone, Debug, Serialize)]
pub struct PeerProgress {
    /// Selected database.
    pub database: DatabaseId,
    /// Authenticated route identity.
    pub peer: PublicKey,
    /// Ciphertext receipt counts or a sanitized transport failure.
    pub result: Result<ExchangeReport, taypeer_sync::Error>,
}
pub(crate) struct Network {
    relay: RelaySetting,
    node: Arc<Node>,
    pump: tokio::task::JoinHandle<()>,
    progress: Arc<Mutex<BTreeMap<(DatabaseId, PublicKey), PeerProgress>>>,
}
impl RuntimeHost {
    /// Start Iroh for this CLI lifetime. A relay is used only when explicitly selected.
    pub fn start_network(&mut self, relay: RelaySetting) -> Result<EndpointAddr, RuntimeError> {
        if let Some(network) = &self.network {
            if network.relay != relay {
                return Err(RuntimeError::Protocol);
            }
            return Ok(network.node.address());
        }
        let routes: BTreeMap<PublicKey, EndpointAddr> =
            self.profile().load_state("routes")?.unwrap_or_default();
        for (key, address) in routes {
            if *address.id.as_bytes() != *key.as_bytes() {
                return Err(RuntimeError::Protocol);
            }
            self.context
                .coordinator
                .remember_route(address)
                .map_err(sync_error)?;
        }
        let backend: Arc<dyn taypeer_sync::Backend> = self.context.coordinator.clone();
        let node = Arc::new(
            self.runtime
                .block_on(Node::bind(&self.context.transport, &relay, backend))
                .map_err(sync_error)?,
        );
        if !matches!(relay, RelaySetting::Disabled) {
            self.runtime.block_on(node.online()).map_err(sync_error)?;
        }
        let progress = Arc::new(Mutex::new(BTreeMap::new()));
        let updates = Arc::clone(&progress);
        let context = Arc::clone(&self.context);
        let endpoint = Arc::clone(&node);
        let pump = self.runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            let mut saved_routes = BTreeMap::new();
            loop {
                interval.tick().await;
                let routes = match context.coordinator.routes() {
                    Ok(routes) => routes,
                    Err(_) => break,
                };
                if routes != saved_routes {
                    let profile = context.profile.clone();
                    let next = routes.clone();
                    let saved =
                        tokio::task::spawn_blocking(move || profile.save_state("routes", &next))
                            .await;
                    if matches!(saved, Ok(Ok(()))) {
                        saved_routes = routes.clone();
                    }
                }
                let databases: Vec<_> = match context.copies.lock() {
                    Ok(copies) => copies.values().map(|r| r.database.clone()).collect(),
                    Err(_) => break,
                };
                let mut tasks = tokio::task::JoinSet::new();
                for database in databases {
                    let snapshot = match context.coordinator.snapshot(&database) {
                        Ok(snapshot) => snapshot,
                        Err(_) => continue,
                    };
                    for (peer, address) in &routes {
                        if *peer == context.transport.public()
                            || snapshot.chain().admit_transport(*peer).is_err()
                        {
                            continue;
                        }
                        let endpoint = Arc::clone(&endpoint);
                        let database = database.clone();
                        let peer = *peer;
                        let address = address.clone();
                        let progress = Arc::clone(&updates);
                        tasks.spawn(async move {
                            let result = endpoint.exchange(address, database.clone()).await;
                            if let Ok(mut progress) = progress.lock() {
                                progress.insert(
                                    (database.clone(), peer),
                                    PeerProgress {
                                        database,
                                        peer,
                                        result,
                                    },
                                );
                            }
                        });
                        if tasks.len() >= 8 {
                            let _ = tasks.join_next().await;
                        }
                    }
                }
                while tasks.join_next().await.is_some() {}
            }
        });
        let address = node.address();
        self.network = Some(Network {
            relay,
            node,
            pump,
            progress,
        });
        Ok(address)
    }
    /// Stop all network tasks without closing the registered ciphertext files or open workers.
    pub fn stop_network(&mut self) {
        if let Some(network) = self.network.take() {
            network.pump.abort();
            self.runtime.block_on(async {
                let _ = network.pump.await;
            });
            if let Ok(mut node) = Arc::try_unwrap(network.node) {
                self.runtime.block_on(node.close());
            }
        }
    }
    /// Local endpoint routes for an invitation or explicit authenticated peer exchange.
    pub fn network_address(&self) -> Result<EndpointAddr, RuntimeError> {
        Ok(self
            .network
            .as_ref()
            .ok_or(RuntimeError::Closed)?
            .node
            .address())
    }
    /// Safe per-peer receipt/error status. Application is reported separately by the worker.
    pub fn network_progress(&self) -> Result<Vec<PeerProgress>, RuntimeError> {
        Ok(self
            .network
            .as_ref()
            .ok_or(RuntimeError::Closed)?
            .progress
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .values()
            .cloned()
            .collect())
    }
    /// Exchange once with an explicitly supplied route. The peer must already be admitted.
    pub fn exchange(
        &self,
        address: EndpointAddr,
        database: DatabaseId,
    ) -> Result<ExchangeReport, RuntimeError> {
        let node = &self.network.as_ref().ok_or(RuntimeError::Closed)?.node;
        self.runtime
            .block_on(node.exchange(address, database))
            .map_err(sync_error)
    }
    /// Present a code through a real connection. Author proof is created in a short-lived
    /// enrollment process, and the bearer is discarded once presentation completes.
    pub fn join(
        &self,
        executable: &Path,
        code: InvitationCode,
        destination: &Path,
    ) -> Result<JoinProgress, RuntimeError> {
        if *code.address.id.as_bytes() != *code.invitation.transport.as_bytes() {
            return Err(RuntimeError::Protocol);
        }
        let destination = canonical_path(destination)?;
        if destination
            .try_exists()
            .map_err(|_| RuntimeError::Transport)?
        {
            return Err(storage(taypeer_storage::Error::AlreadyExists));
        }
        let proof = Worker::enroll(executable, self.profile(), code.invitation.clone())?;
        let pending = PendingJoin {
            invitation: code.invitation.clone(),
            proof: proof.clone(),
            address: code.address.clone(),
            path: destination,
        };
        let request = code.invitation.id().map_err(|_| RuntimeError::Protocol)?;
        let mut joins = self.pending_joins()?;
        if let Some(existing) = joins.get(&request)
            && (existing.path != pending.path || existing.proof != pending.proof)
        {
            return Err(RuntimeError::Protocol);
        }
        joins.insert(request, pending);
        self.profile().save_state("joins", &joins)?;
        let node = &self.network.as_ref().ok_or(RuntimeError::Closed)?.node;
        let command = Command::Join {
            invitation: Box::new(code.invitation),
            secret: code.secret,
            proof: Box::new(proof),
            address: node.address(),
        };
        match self
            .runtime
            .block_on(node.request(code.address, command))
            .map_err(sync_error)?
        {
            Reply::JoinPending(id) if id == request => Ok(JoinProgress::Pending(id)),
            _ => Err(RuntimeError::Protocol),
        }
    }
    /// Public enrollment requests retained through host restart, without their bearer secrets.
    pub fn pending_joins(&self) -> Result<BTreeMap<Digest, PendingJoin>, RuntimeError> {
        Ok(self.profile().load_state("joins")?.unwrap_or_default())
    }
    /// Resume approval/download using authenticated recipient identity, including after code expiry.
    /// Each fully fetched object is durably staged and reused after interruption.
    pub fn resume_join(&self, request: Digest) -> Result<JoinProgress, RuntimeError> {
        let mut joins = self.pending_joins()?;
        let pending = joins.get(&request).ok_or(RuntimeError::Protocol)?.clone();
        let node = &self.network.as_ref().ok_or(RuntimeError::Closed)?.node;
        let command = Command::JoinStatus {
            database: pending.invitation.database.clone(),
            request,
        };
        let metadata = match self
            .runtime
            .block_on(node.request(pending.address.clone(), command))
            .map_err(sync_error)?
        {
            Reply::JoinPending(id) if id == request => return Ok(JoinProgress::Pending(id)),
            Reply::JoinRejected => return Ok(JoinProgress::Rejected),
            Reply::Joined(metadata) => *metadata,
            _ => return Err(RuntimeError::Protocol),
        };
        let database = self.download_join(request, &pending, metadata)?;
        self.context
            .coordinator
            .remember_route(pending.address)
            .map_err(sync_error)?;
        self.profile().save_state(
            "routes",
            &self.context.coordinator.routes().map_err(sync_error)?,
        )?;
        joins.remove(&request);
        self.profile().save_state("joins", &joins)?;
        Ok(JoinProgress::Received(database))
    }
    fn download_join(
        &self,
        request: Digest,
        pending: &PendingJoin,
        metadata: ArchiveMetadata,
    ) -> Result<DatabaseId, RuntimeError> {
        let chain = metadata.verify(pending.invitation.root).map_err(storage)?;
        let sequence = chain
            .at(pending.invitation.control)
            .map_err(|_| RuntimeError::Protocol)?
            .sequence;
        let issuing = taypeer_trust::ControlChain::validate(
            chain
                .records()
                .iter()
                .take_while(|c| c.body.sequence <= sequence)
                .cloned()
                .collect(),
            pending.invitation.root,
        )
        .map_err(|_| RuntimeError::Protocol)?;
        pending
            .invitation
            .verify(&issuing, pending.invitation.issued_at)
            .map_err(|_| RuntimeError::Protocol)?;
        if chain
            .head()
            .members
            .get(&pending.proof.recipient.device)
            .is_none_or(|m| m.identity != pending.proof.recipient)
        {
            return Err(RuntimeError::Protocol);
        }
        if pending
            .path
            .try_exists()
            .map_err(|_| RuntimeError::Transport)?
        {
            // Lost final local response: only an already protected matching registration
            // can make an existing destination an idempotent continuation.
            let registration = self
                .profile()
                .registration(&pending.path)?
                .ok_or(RuntimeError::Protocol)?;
            if registration.root != pending.invitation.root {
                return Err(RuntimeError::Protocol);
            }
            self.context.attach(&pending.path)?;
            return Ok(registration.database);
        }
        let directory = self.profile().directory().join(format!("join-{request}"));
        std::fs::create_dir_all(&directory).map_err(|_| RuntimeError::Transport)?;
        let node = &self.network.as_ref().ok_or(RuntimeError::Closed)?.node;
        let mut candidate = ArchiveCandidate::new();
        for descriptor in metadata.manifest.body.objects.values() {
            let path = directory.join(descriptor.digest.to_string());
            let object = if path.try_exists().map_err(|_| RuntimeError::Transport)? {
                EncryptedObject::receive(
                    File::open(&path).map_err(|_| RuntimeError::Transport)?,
                    descriptor,
                    &chain,
                )
                .map_err(storage)?
            } else {
                let incoming = self
                    .runtime
                    .block_on(node.download_object(
                        pending.address.clone(),
                        pending.invitation.database.clone(),
                        descriptor.clone(),
                    ))
                    .map_err(sync_error)?;
                let object = EncryptedObject::receive(
                    incoming.reopen().map_err(|_| RuntimeError::Transport)?,
                    descriptor,
                    &chain,
                )
                .map_err(storage)?;
                let mut durable = tempfile::NamedTempFile::new_in(&directory)
                    .map_err(|_| RuntimeError::Transport)?;
                std::io::copy(&mut object.reader().map_err(storage)?, &mut durable)
                    .map_err(|_| RuntimeError::Transport)?;
                durable
                    .as_file()
                    .sync_all()
                    .map_err(|_| RuntimeError::Transport)?;
                durable
                    .persist_noclobber(&path)
                    .map_err(|_| RuntimeError::Transport)?;
                File::open(&directory)
                    .and_then(|file| file.sync_all())
                    .map_err(|_| RuntimeError::Transport)?;
                object
            };
            candidate.insert(object).map_err(storage)?;
        }
        let mut body = metadata.manifest.body;
        body.generation = 0;
        let signed = candidate
            .metadata(&chain, &self.context.transport, body, metadata.journal)
            .map_err(storage)?;
        let registration = self.profile().prepare_registration(
            &pending.path,
            chain.head().database.clone(),
            pending.invitation.root,
        )?;
        let store = ArchiveStore::create(
            &pending.path,
            &candidate,
            signed,
            Some(self.profile().anchor(&registration)),
        )
        .map_err(storage)?;
        self.context
            .coordinator
            .register(store)
            .map_err(sync_error)?;
        self.context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .insert(pending.path.clone(), registration.clone());
        Ok(registration.database)
    }
}
impl Drop for RuntimeHost {
    fn drop(&mut self) {
        self.stop_network();
    }
}
