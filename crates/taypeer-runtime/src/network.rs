//! Explicit network lifetime, authenticated routes and resumable invitation downloads.
mod view;
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
pub use view::{DatabaseExchange, DeviceExchange, NetworkCancellation, NetworkSnapshot};
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
/// Nonsecret index entry; private invitation details remain in password-encrypted staging.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PendingJoinSummary {
    /// Database whose enrollment can be resumed after authenticating its staging.
    pub database: DatabaseId,
    /// Explicit destination selected by the user.
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
    wake: Arc<tokio::sync::Notify>,
}
impl RuntimeHost {
    /// Start Iroh for this CLI lifetime. A relay is used only when explicitly selected.
    pub fn start_network(&self, relay: RelaySetting) -> Result<EndpointAddr, RuntimeError> {
        *self
            .requested_relay
            .lock()
            .map_err(|_| RuntimeError::Transport)? = Some(relay.clone());
        let contexts = self.context.active_contexts()?;
        let mut first = None;
        for context in contexts {
            if !context.profile.is_persistent() {
                continue;
            }
            let address = self.start_context_network(context, relay.clone())?;
            first.get_or_insert(address);
        }
        first.ok_or(RuntimeError::Closed)
    }
    fn start_context_network(
        &self,
        context: Arc<crate::host::HostContext>,
        relay: RelaySetting,
    ) -> Result<EndpointAddr, RuntimeError> {
        let mut slot = self.network.lock().map_err(|_| RuntimeError::Transport)?;
        let key = context.transport.public();
        if let Some(network) = slot.get(&key) {
            if network.relay != relay {
                return Err(RuntimeError::Protocol);
            }
            return Ok(network.node.address());
        }
        let routes: BTreeMap<PublicKey, EndpointAddr> =
            context.profile.load_state("routes")?.unwrap_or_default();
        for (key, address) in routes {
            if *address.id.as_bytes() != *key.as_bytes() {
                return Err(RuntimeError::Protocol);
            }
            context
                .coordinator
                .remember_route(address)
                .map_err(sync_error)?;
        }
        let backend: Arc<dyn taypeer_sync::Backend> = context.coordinator.clone();
        let node = Arc::new(
            self.runtime
                .block_on(Node::bind(&context.transport, &relay, backend))
                .map_err(sync_error)?,
        );
        if !matches!(relay, RelaySetting::Disabled) {
            self.runtime.block_on(node.online()).map_err(sync_error)?;
        }
        let progress = Arc::new(Mutex::new(BTreeMap::new()));
        let updates = Arc::clone(&progress);
        let endpoint = Arc::clone(&node);
        let wake = Arc::new(tokio::sync::Notify::new());
        let triggered = Arc::clone(&wake);
        let mut events = context.coordinator.subscribe();
        let pump = self.runtime.spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(2));
            let mut saved_routes = BTreeMap::new();
            loop {
                tokio::select! {
                    _ = interval.tick() => {},
                    _ = triggered.notified() => {},
                    event = events.recv() => {
                        match event {
                            // Reconcile inventory, authority and routes even if
                            // the wakeup that carried their change was lost.
                            Err(tokio::sync::broadcast::error::RecvError::Lagged(_)) => {},
                            Err(tokio::sync::broadcast::error::RecvError::Closed) => break,
                            Ok(_) => {},
                        }
                    }
                }
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
        slot.insert(
            key,
            Network {
                relay,
                node,
                pump,
                progress,
                wake,
            },
        );
        Ok(address)
    }
    /// Stop all network tasks without closing the registered ciphertext files or open workers.
    pub fn stop_network(&self) {
        if let Ok(mut requested) = self.requested_relay.lock() {
            *requested = None;
        }
        let networks = self
            .network
            .lock()
            .map(|mut slot| std::mem::take(&mut *slot))
            .unwrap_or_default();
        for (_, network) in networks {
            network.pump.abort();
            self.runtime.block_on(async {
                let _ = network.pump.await;
                network.node.close().await;
            });
        }
    }
    fn network_node(&self) -> Result<Arc<Node>, RuntimeError> {
        self.network
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .values()
            .next()
            .map(|network| Arc::clone(&network.node))
            .ok_or(RuntimeError::Closed)
    }
    pub(crate) fn node_for(&self, database: &DatabaseId) -> Result<Arc<Node>, RuntimeError> {
        let context = self.context.for_database(database)?;
        self.network
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .get(&context.transport.public())
            .map(|network| Arc::clone(&network.node))
            .ok_or(RuntimeError::Closed)
    }
    /// Local endpoint routes for an invitation or explicit authenticated peer exchange.
    pub fn network_address(&self) -> Result<EndpointAddr, RuntimeError> {
        Ok(self.network_node()?.address())
    }
    /// Endpoint of the independently activated transport for a selected database.
    pub fn network_address_for(&self, database: &DatabaseId) -> Result<EndpointAddr, RuntimeError> {
        Ok(self.node_for(database)?.address())
    }
    /// Safe per-peer receipt/error status. Application is reported separately by the worker.
    pub fn network_progress(&self) -> Result<Vec<PeerProgress>, RuntimeError> {
        let progress: Vec<_> = self
            .network
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .values()
            .map(|network| Arc::clone(&network.progress))
            .collect();
        let mut values = Vec::new();
        for progress in progress {
            values.extend(
                progress
                    .lock()
                    .map_err(|_| RuntimeError::Transport)?
                    .values()
                    .cloned(),
            );
        }
        Ok(values)
    }
    /// Exchange once with an explicitly supplied route. The peer must already be admitted.
    pub fn exchange(
        &self,
        address: EndpointAddr,
        database: DatabaseId,
    ) -> Result<ExchangeReport, RuntimeError> {
        let node = self.node_for(&database)?;
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
        password: String,
    ) -> Result<JoinProgress, RuntimeError> {
        self.join_cancellable(
            executable,
            code,
            destination,
            password,
            &NetworkCancellation::default(),
        )
    }
    /// Present an invitation with cancellation. A persisted request can be resumed after interruption.
    pub fn join_cancellable(
        &self,
        executable: &Path,
        code: InvitationCode,
        destination: &Path,
        password: String,
        cancellation: &NetworkCancellation,
    ) -> Result<JoinProgress, RuntimeError> {
        let password = Zeroizing::new(password);
        cancellation.check()?;
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
        let (proof, enrollment) = Worker::enroll(
            executable,
            Arc::clone(&self.context),
            &destination,
            password.to_string(),
            code.invitation.clone(),
            &self.sessions,
        )?;
        let pending = PendingJoin {
            invitation: code.invitation.clone(),
            proof: proof.clone(),
            address: code.address.clone(),
            path: destination,
        };
        let request = code.invitation.id().map_err(|_| RuntimeError::Protocol)?;
        if self.profile().is_linux_lazy() {
            self.enrollments
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .insert(request, enrollment);
        } else {
            enrollment
                .control
                .invalidate(crate::session::LockReason::Manual);
            enrollment.control.wait_closed()?;
        }
        let mut joins = self.pending_joins()?;
        if let Some(existing) = joins.get(&request)
            && (existing.path != pending.path || existing.proof != pending.proof)
        {
            return Err(RuntimeError::Protocol);
        }
        if self.profile().is_linux_lazy() {
            self.profile()
                .save_join_request(request, &pending, password.as_bytes())?;
        }
        joins.insert(request, pending);
        let context = self.context.for_database(&code.invitation.database)?;
        let local: BTreeMap<_, _> = joins
            .iter()
            .filter(|(_, pending)| pending.invitation.database == code.invitation.database)
            .map(|(id, pending)| (*id, pending.clone()))
            .collect();
        context.profile.save_state("joins", &local)?;
        let relay = self
            .requested_relay
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .clone()
            .unwrap_or(RelaySetting::Disabled);
        self.start_network(relay)?;
        let node = self.node_for(&code.invitation.database)?;
        let command = Command::Join {
            invitation: Box::new(code.invitation),
            secret: code.secret,
            proof: Box::new(proof),
            address: node.address(),
        };
        match self.runtime.block_on(cancellation.run(async {
            node.request(code.address, command)
                .await
                .map_err(sync_error)
        }))? {
            Reply::JoinPending(id) if id == request => Ok(JoinProgress::Pending(id)),
            _ => Err(RuntimeError::Protocol),
        }
    }
    /// Public enrollment requests retained through host restart, without their bearer secrets.
    pub fn pending_joins(&self) -> Result<BTreeMap<Digest, PendingJoin>, RuntimeError> {
        let mut joins = BTreeMap::new();
        for context in self.context.active_contexts()? {
            joins.extend(
                context
                    .profile
                    .load_state::<BTreeMap<Digest, PendingJoin>>("joins")?
                    .unwrap_or_default(),
            );
        }
        Ok(joins)
    }
    /// List resumable requests without acquiring their author/transport credentials.
    pub fn pending_join_summaries(
        &self,
    ) -> Result<BTreeMap<Digest, PendingJoinSummary>, RuntimeError> {
        if self.profile().is_linux_lazy() {
            return self.profile().join_summaries().map_err(RuntimeError::from);
        }
        Ok(self
            .pending_joins()?
            .into_iter()
            .map(|(request, pending)| {
                (
                    request,
                    PendingJoinSummary {
                        database: pending.invitation.database,
                        path: pending.path,
                    },
                )
            })
            .collect())
    }
    /// Resume approval/download using authenticated recipient identity, including after code expiry.
    /// Each fully fetched object is durably staged and reused after interruption.
    pub fn resume_join(
        &self,
        request: Digest,
        password: String,
    ) -> Result<JoinProgress, RuntimeError> {
        self.resume_join_cancellable(request, password, &NetworkCancellation::default())
    }
    /// Resume a download; cancellation keeps completely staged ciphertext for the next attempt.
    pub fn resume_join_cancellable(
        &self,
        request: Digest,
        password: String,
        cancellation: &NetworkCancellation,
    ) -> Result<JoinProgress, RuntimeError> {
        let password = Zeroizing::new(password);
        cancellation.check()?;
        let mut joins = self.pending_joins()?;
        let active_enrollment = self
            .enrollments
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .get(&request)
            .is_some_and(|client| client.control.check(false).is_ok());
        let pending = if let Some(pending) = joins
            .get(&request)
            .filter(|_| !self.profile().is_linux_lazy() || active_enrollment)
        {
            pending.clone()
        } else if self.profile().is_linux_lazy() {
            let pending = self
                .profile()
                .restore_join_request(request, password.as_bytes())?;
            let executable = std::env::current_exe().map_err(|_| RuntimeError::Transport)?;
            let (proof, enrollment) = Worker::enroll(
                &executable,
                Arc::clone(&self.context),
                &pending.path,
                password.to_string(),
                pending.invitation.clone(),
                &self.sessions,
            )?;
            if proof != pending.proof {
                return Err(RuntimeError::Protocol);
            }
            self.enrollments
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .insert(request, enrollment);
            joins.insert(request, pending.clone());
            let context = self.context.for_database(&pending.invitation.database)?;
            let local: BTreeMap<_, _> = joins
                .iter()
                .filter(|(_, other)| other.invitation.database == pending.invitation.database)
                .map(|(id, pending)| (*id, pending.clone()))
                .collect();
            context.profile.save_state("joins", &local)?;
            let relay = self
                .requested_relay
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .clone()
                .unwrap_or(RelaySetting::Disabled);
            self.start_network(relay)?;
            pending
        } else {
            return Err(RuntimeError::Protocol);
        };
        let context = self.context.for_database(&pending.invitation.database)?;
        let node = self.node_for(&pending.invitation.database)?;
        let command = Command::JoinStatus {
            database: pending.invitation.database.clone(),
            request,
        };
        let metadata = match self.runtime.block_on(cancellation.run(async {
            node.request(pending.address.clone(), command)
                .await
                .map_err(sync_error)
        }))? {
            Reply::JoinPending(id) if id == request => return Ok(JoinProgress::Pending(id)),
            Reply::JoinRejected => return Ok(JoinProgress::Rejected),
            Reply::Joined(metadata) => *metadata,
            _ => return Err(RuntimeError::Protocol),
        };
        let database = self.download_join(request, &pending, metadata, cancellation)?;
        if self.profile().is_linux_lazy() {
            // An incorrect password preserves ciphertext and the pending request for retry.
            let enrollment = self
                .enrollments
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .get(&request)
                .cloned()
                .ok_or(RuntimeError::Closed)?;
            enrollment.request(
                &crate::Command::BindInvitation {
                    path: pending.path.clone(),
                    password: Zeroizing::new(password.as_bytes().to_vec()),
                },
                false,
            )?;
            enrollment
                .control
                .invalidate(crate::session::LockReason::Manual);
            enrollment.control.wait_closed()?;
            self.enrollments
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .remove(&request);
        }
        context
            .coordinator
            .remember_route(pending.address)
            .map_err(sync_error)?;
        context
            .profile
            .save_state("routes", &context.coordinator.routes().map_err(sync_error)?)?;
        // Keep the resumable request until both ciphertext and the common catalog
        // are durable. A lost catalog response retries this same registered path.
        self.register_working_copy(&pending.path)?;
        joins.remove(&request);
        let local: BTreeMap<_, _> = joins
            .iter()
            .filter(|(_, other)| other.invitation.database == pending.invitation.database)
            .map(|(id, pending)| (*id, pending.clone()))
            .collect();
        context.profile.save_state("joins", &local)?;
        if self.profile().is_linux_lazy() {
            self.profile().finish_join_request(request)?;
        }
        Ok(JoinProgress::Received(database))
    }
    fn download_join(
        &self,
        request: Digest,
        pending: &PendingJoin,
        metadata: ArchiveMetadata,
        cancellation: &NetworkCancellation,
    ) -> Result<DatabaseId, RuntimeError> {
        let context = self.context.for_database(&pending.invitation.database)?;
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
            let registration = context
                .profile
                .registration(&pending.path)?
                .ok_or(RuntimeError::Protocol)?;
            if registration.root != pending.invitation.root {
                return Err(RuntimeError::Protocol);
            }
            context.attach(&pending.path)?;
            return Ok(registration.database);
        }
        let directory = context.profile.directory().join(format!("join-{request}"));
        use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&directory)
            .map_err(|_| RuntimeError::Transport)?;
        std::fs::set_permissions(&directory, std::fs::Permissions::from_mode(0o700))
            .map_err(|_| RuntimeError::Transport)?;
        let node = self.node_for(&pending.invitation.database)?;
        let mut candidate = ArchiveCandidate::new();
        for descriptor in metadata.manifest.body.objects.values() {
            cancellation.check()?;
            let path = directory.join(descriptor.digest.to_string());
            let object = if path.try_exists().map_err(|_| RuntimeError::Transport)? {
                EncryptedObject::receive(
                    File::open(&path).map_err(|_| RuntimeError::Transport)?,
                    descriptor,
                    &chain,
                )
                .map_err(storage)?
            } else {
                let incoming = self.runtime.block_on(cancellation.run(async {
                    node.download_object(
                        pending.address.clone(),
                        pending.invitation.database.clone(),
                        descriptor.clone(),
                    )
                    .await
                    .map_err(sync_error)
                }))?;
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
                taypeer_storage::publish_file(
                    durable,
                    &path,
                    taypeer_storage::PublicationMode::Create,
                )
                .map_err(storage)?;
                object
            };
            candidate.insert(object).map_err(storage)?;
        }
        cancellation.check()?;
        let mut body = metadata.manifest.body;
        body.generation = 0;
        let signed = candidate
            .metadata(&chain, &context.transport, body, metadata.journal)
            .map_err(storage)?;
        let registration = context.profile.prepare_registration(
            &pending.path,
            chain.head().database.clone(),
            pending.invitation.root,
        )?;
        let store = ArchiveStore::create(
            &pending.path,
            &candidate,
            signed,
            Some(context.profile.anchor(&registration)),
        )
        .map_err(storage)?;
        context.coordinator.register(store).map_err(sync_error)?;
        context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .insert(pending.path.clone(), registration.clone());
        Ok(registration.database)
    }
}
impl Drop for RuntimeHost {
    fn drop(&mut self) {
        self.sessions
            .lock_all(crate::session::LockReason::HostExited);
        self.stop_network();
    }
}
