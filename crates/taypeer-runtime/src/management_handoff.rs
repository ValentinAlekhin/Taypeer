//! Explicit two-sided management decisions, owned by the ciphertext host without author keys.
use crate::{
    Command, RuntimeError,
    host::{HostContext, RuntimeHost, sync_error},
    process::Client,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeMap,
    sync::{Arc, Mutex, Weak},
};
use taypeer_core::DatabaseId;
use taypeer_services::OpenManagementBasis;
use taypeer_storage::{ArchiveSnapshot, CiphertextFile, ObjectReader};
use taypeer_sync::{
    Backend, ManagementApplied, ManagementDecision, ManagementIntent, ManagementReadiness,
    ManagementReply,
};
use taypeer_trust::{CipherObject, DeviceId, Digest, PublicKey};

mod drive;
mod store;
use store::{Direction, Phase, Record, Store};

/// Content-free marker for one actual OPEN worker registered with this host.
/// This marker is not a secret capability: every use checks the registered generation again.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementSession {
    /// Selected logical database.
    pub database: DatabaseId,
    /// Actual process generation; a later unlock cannot reuse it.
    pub generation: u64,
}
/// Last confirmed local phase of an immutable management operation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementPhase {
    /// Source intent is durable; recipient has not yet acknowledged its proposal.
    Proposed,
    /// Recipient proposal is durable; explicit decision/signing remains pending.
    WaitingDecision,
    /// Recipient's explicit acceptance is durable; exact signing/confirmation must resume.
    /// Rejection and a different intent cannot replace this retained decision.
    AcceptRequested,
    /// Exact recipient consent is durably recorded, without granting its applied role.
    ConsentRecorded,
    /// Explicit recipient rejection is durable.
    Rejected,
    /// Source control/baseline publication is confirmed; old manager has relinquished its role.
    SourceCommitted,
    /// Ciphertext delivery succeeded; recipient application acknowledgement is still pending.
    AwaitingApply,
    /// Related recipient application and this local phase receipt are confirmed.
    Completed,
}
/// Public workflow row; signed consent and private worker handles never enter the UI DTO.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementHandoff {
    /// Original exact user-approved intent.
    pub intent: ManagementIntent,
    /// Last durably confirmed local phase, independent of peer availability.
    pub phase: ManagementPhase,
    /// Actual applied result only after durable completion.
    pub applied: Option<ManagementApplied>,
}
/// One fresh authenticated peer observation; the signed private decision is not exposed.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagementPeerStatus {
    /// Original correlated intent.
    pub intent: ManagementIntent,
    /// Peer durable phase, absent when no exact proposal/decision exists there.
    pub phase: Option<ManagementPhase>,
    /// Actual live worker observation, valid only for this RPC.
    pub readiness: ManagementReadiness,
}
/// Sanitized failures for a client to localize without examining strings or wire payloads.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ManagementFailure {
    /// No matching actual OPEN worker exists; no authentication is performed automatically.
    NeedsUnlock,
    /// The actual worker has not durably applied current control and baseline yet.
    PendingApply,
    /// The chosen head or membership interval changed before the transition.
    Stale,
    /// This action belongs to the opposite source/recipient or to another current role.
    WrongRole,
    /// No exact protected intent exists for retry/accept/reject.
    Unknown,
    /// An operation identity is already bound to another intent or decision.
    Conflict,
    /// The bounded local receipt store has no room for a new operation.
    Capacity,
    /// The peer/backend does not implement the explicit management workflow.
    Unsupported,
}
impl std::fmt::Debug for ManagementSession {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementSession([REDACTED])")
    }
}
impl std::fmt::Debug for ManagementHandoff {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementHandoff([REDACTED])")
    }
}
impl std::fmt::Debug for ManagementPeerStatus {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("ManagementPeerStatus([REDACTED])")
    }
}
fn failure(reason: ManagementFailure) -> RuntimeError {
    RuntimeError::Management(reason)
}

impl crate::Worker {
    /// Bind one explicit management action to this actual OPEN process generation.
    /// The host rechecks its weak registered command capability for every action.
    pub fn management_session(&self) -> Result<ManagementSession, RuntimeError> {
        self.client.control.check(false)?;
        Ok(ManagementSession {
            database: self.database.clone(),
            generation: self.client.control.status().generation,
        })
    }
}

#[derive(Default)]
pub(crate) struct Owner {
    store: Store,
    // No strong process ownership: lock/close stays independent of pending network work.
    workers: Mutex<BTreeMap<DatabaseId, Weak<Client>>>,
}
struct LiveWorker {
    client: Arc<Client>,
    basis: OpenManagementBasis,
    generation: u64,
}
impl Owner {
    pub(crate) fn register(
        &self,
        database: DatabaseId,
        client: &Arc<Client>,
    ) -> Result<(), RuntimeError> {
        client.control.check(false)?;
        self.workers
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .insert(database, Arc::downgrade(client));
        Ok(())
    }
    fn client(
        &self,
        database: &DatabaseId,
        generation: Option<u64>,
    ) -> Result<Arc<Client>, RuntimeError> {
        let client = self
            .workers
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .get(database)
            .and_then(Weak::upgrade)
            .ok_or_else(|| failure(ManagementFailure::NeedsUnlock))?;
        client.control.check(false)?;
        let status = client.control.status();
        if status.database.as_ref() != Some(database)
            || generation.is_some_and(|expected| expected != status.generation)
        {
            return Err(failure(ManagementFailure::NeedsUnlock));
        }
        Ok(client)
    }
    fn live(
        &self,
        database: &DatabaseId,
        generation: Option<u64>,
    ) -> Result<LiveWorker, RuntimeError> {
        let client = self.client(database, generation)?;
        let status = client.control.status();
        let basis: Option<OpenManagementBasis> =
            serde_json::from_value(client.request(&Command::ManagementBasis, false)?)
                .map_err(|_| RuntimeError::Protocol)?;
        let basis = basis.ok_or_else(|| failure(ManagementFailure::PendingApply))?;
        client.control.check(false)?;
        if basis.database != *database {
            return Err(RuntimeError::Protocol);
        }
        Ok(LiveWorker {
            client,
            basis,
            generation: status.generation,
        })
    }
    fn registration(
        context: &HostContext,
        database: &DatabaseId,
    ) -> Result<crate::profile::Registration, RuntimeError> {
        context
            .copies
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .values()
            .find(|registration| &registration.database == database)
            .cloned()
            .ok_or(RuntimeError::Closed)
    }
    fn snapshot(
        context: &HostContext,
        intent: &ManagementIntent,
    ) -> Result<ArchiveSnapshot, RuntimeError> {
        let state = context
            .coordinator
            .state(&intent.database)
            .map_err(sync_error)?;
        if state.frozen {
            return Err(taypeer_services::ServiceError::Trust(taypeer_trust::Error::Fork).into());
        }
        let snapshot = context
            .coordinator
            .snapshot(&intent.database)
            .map_err(sync_error)?;
        intent.verify(snapshot.chain()).map_err(sync_error)?;
        Ok(snapshot)
    }
    fn authenticate(
        context: &HostContext,
        intent: &ManagementIntent,
        peer: PublicKey,
    ) -> Result<Direction, RuntimeError> {
        let snapshot = Self::snapshot(context, intent)?;
        let chain = snapshot.chain();
        let local = chain
            .admit_transport(context.transport.public())
            .map_err(|_| failure(ManagementFailure::WrongRole))?;
        let remote = chain
            .admit_transport(peer)
            .map_err(|_| failure(ManagementFailure::WrongRole))?;
        let direction = if local == intent.manager && remote == intent.recipient {
            Direction::Source
        } else if local == intent.recipient && remote == intent.manager {
            Direction::Recipient
        } else {
            return Err(failure(ManagementFailure::WrongRole));
        };
        let previous = chain
            .at(intent.previous)
            .map_err(|_| failure(ManagementFailure::Stale))?;
        if previous
            .members
            .get(&local)
            .is_none_or(|member| member.identity.transport != context.transport.public())
            || previous
                .members
                .get(&remote)
                .is_none_or(|member| member.identity.transport != peer)
        {
            return Err(failure(ManagementFailure::WrongRole));
        }
        Ok(direction)
    }
    fn readiness(
        &self,
        context: &HostContext,
        intent: &ManagementIntent,
    ) -> Result<ManagementReadiness, RuntimeError> {
        let live = match self.live(&intent.database, None) {
            Ok(live) => live,
            Err(
                RuntimeError::Management(ManagementFailure::NeedsUnlock)
                | RuntimeError::Closed
                | RuntimeError::SessionClosed(_)
                | RuntimeError::OperationInterrupted(_),
            ) => return Ok(ManagementReadiness::Locked),
            Err(RuntimeError::Management(ManagementFailure::PendingApply)) => {
                return Ok(ManagementReadiness::PendingApply);
            }
            Err(error) => return Err(error),
        };
        if live.basis.root != intent.root
            || live.basis.identity.transport != context.transport.public()
        {
            return Err(RuntimeError::Protocol);
        }
        Ok(ManagementReadiness::Ready {
            generation: live.generation,
            identity: live.basis.identity,
            control: live.basis.control,
            baseline: live.basis.baseline,
        })
    }
    fn reply(
        &self,
        context: &HostContext,
        intent: ManagementIntent,
        challenge: Digest,
        direction: Direction,
    ) -> Result<ManagementReply, RuntimeError> {
        let registration = Self::registration(context, &intent.database)?;
        let record = self
            .store
            .record(&context.profile, &registration, &intent, true)?;
        if record
            .as_ref()
            .is_some_and(|record| record.direction != direction)
        {
            return Err(failure(ManagementFailure::WrongRole));
        }
        let readiness = self.readiness(context, &intent)?;
        let mut decision = record
            .as_ref()
            .map_or(ManagementDecision::Unknown, Record::decision);
        if direction == Direction::Recipient
            && matches!(decision, ManagementDecision::Accepted(_))
            && let ManagementReadiness::Ready {
                control, baseline, ..
            } = &readiness
        {
            let snapshot = Self::snapshot(context, &intent)?;
            if let Some(transition) = intent
                .committed_control(snapshot.chain())
                .map_err(sync_error)?
                && applied_contains(snapshot.chain(), transition, *control)?
            {
                let applied = ManagementApplied {
                    intent: intent.clone(),
                    transition,
                    control: *control,
                    baseline: *baseline,
                };
                let record =
                    self.store
                        .update(&context.profile, &registration, &intent, |existing| {
                            let mut record =
                                existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                            match &record.phase {
                                Phase::ConsentRecorded(_) => {
                                    record.phase = Phase::Completed(applied.clone())
                                }
                                Phase::Completed(_) => {}
                                _ => return Err(failure(ManagementFailure::Conflict)),
                            }
                            Ok(record)
                        })?;
                decision = record.decision();
            }
        }
        // A profile ACK may outlive the worker. Recheck actual access for the readiness
        // included in this RPC; the durable decision itself remains useful when locked.
        let readiness = self.readiness(context, &intent)?;
        Ok(ManagementReply {
            intent,
            challenge,
            decision,
            readiness,
        })
    }
    fn command(
        &self,
        context: &HostContext,
        peer: PublicKey,
        command: taypeer_sync::Command,
    ) -> Result<taypeer_sync::Reply, RuntimeError> {
        let (intent, challenge, address) = match command {
            taypeer_sync::Command::ManagementProposal {
                intent,
                challenge,
                address,
            } => (intent, challenge, Some(address)),
            taypeer_sync::Command::ManagementStatus { intent, challenge } => {
                (intent, challenge, None)
            }
            _ => return Err(RuntimeError::Protocol),
        };
        if serde_json::to_vec(&intent)
            .map_err(|_| RuntimeError::Protocol)?
            .len()
            > 4096
        {
            return Err(RuntimeError::TooLarge);
        }
        let direction = Self::authenticate(context, &intent, peer)?;
        if let Some(address) = address {
            if direction != Direction::Recipient || *address.id.as_bytes() != *peer.as_bytes() {
                return Err(failure(ManagementFailure::WrongRole));
            }
            let registration = Self::registration(context, &intent.database)?;
            let existing = self
                .store
                .record(&context.profile, &registration, &intent, false)?;
            if existing.is_none() {
                let live = match self.live(&intent.database, None) {
                    Ok(live) => live,
                    Err(
                        RuntimeError::Management(
                            ManagementFailure::NeedsUnlock | ManagementFailure::PendingApply,
                        )
                        | RuntimeError::Closed
                        | RuntimeError::SessionClosed(_)
                        | RuntimeError::OperationInterrupted(_),
                    ) => {
                        return Ok(taypeer_sync::Reply::Management(Box::new(
                            self.reply(context, intent, challenge, direction)?,
                        )));
                    }
                    Err(error) => return Err(error),
                };
                require_previous(&live, &intent, Direction::Recipient)?;
                self.store
                    .update(
                        &context.profile,
                        &registration,
                        &intent,
                        |existing| match existing {
                            Some(record) if record.direction == direction => Ok(record),
                            Some(_) => Err(failure(ManagementFailure::WrongRole)),
                            None => Ok(Record {
                                intent: intent.clone(),
                                direction,
                                phase: Phase::WaitingDecision,
                            }),
                        },
                    )?;
                // Lock after protected publication retains the proposal but revokes Ready.
                // reply() reads the independent control again before sending its status.
            }
            context
                .coordinator
                .remember_route(address)
                .map_err(sync_error)?;
            context
                .profile
                .save_state("routes", &context.coordinator.routes().map_err(sync_error)?)?;
        }
        Ok(taypeer_sync::Reply::Management(Box::new(
            self.reply(context, intent, challenge, direction)?,
        )))
    }
}

fn applied_contains(
    chain: &taypeer_trust::ControlChain,
    transition: Digest,
    applied: Digest,
) -> Result<bool, RuntimeError> {
    let transition = chain.at(transition).map_err(|_| RuntimeError::Protocol)?;
    let applied = chain.at(applied).map_err(|_| RuntimeError::Protocol)?;
    // The basis may have been read immediately before a newer head arrived.
    // Its own applied authority must contain this transition; receipt alone is insufficient.
    Ok(applied.sequence >= transition.sequence)
}

fn require_previous(
    live: &LiveWorker,
    intent: &ManagementIntent,
    direction: Direction,
) -> Result<(), RuntimeError> {
    if live.basis.root != intent.root
        || live.basis.control != intent.previous
        || live.basis.manager != intent.manager
    {
        return Err(failure(ManagementFailure::Stale));
    }
    let device = match direction {
        Direction::Source => intent.manager,
        Direction::Recipient => intent.recipient,
    };
    if live.basis.identity.device != device {
        return Err(failure(ManagementFailure::WrongRole));
    }
    live.client.control.check(false)
}

pub(crate) struct NetworkBackend {
    pub context: Arc<HostContext>,
}
impl Backend for NetworkBackend {
    fn command(
        &self,
        peer: PublicKey,
        command: taypeer_sync::Command,
    ) -> Result<taypeer_sync::Reply, taypeer_sync::Error> {
        if matches!(
            &command,
            taypeer_sync::Command::ManagementProposal { .. }
                | taypeer_sync::Command::ManagementStatus { .. }
        ) {
            self.context
                .management
                .command(&self.context, peer, command)
                .map_err(network_error)
        } else {
            self.context.coordinator.command(peer, command)
        }
    }
    fn authorize_object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: &CipherObject,
    ) -> Result<(), taypeer_sync::Error> {
        self.context
            .coordinator
            .authorize_object(peer, database, descriptor)
    }
    fn object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        id: Digest,
    ) -> Result<(CipherObject, ObjectReader), taypeer_sync::Error> {
        self.context.coordinator.object(peer, database, id)
    }
    fn receive(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: CipherObject,
        file: CiphertextFile,
    ) -> Result<Digest, taypeer_sync::Error> {
        self.context
            .coordinator
            .receive(peer, database, descriptor, file)
    }
}
fn network_error(error: RuntimeError) -> taypeer_sync::Error {
    match error {
        RuntimeError::Service(taypeer_services::ServiceError::Storage(error)) => {
            taypeer_sync::Error::Storage(error)
        }
        RuntimeError::Service(taypeer_services::ServiceError::Trust(error)) => {
            taypeer_sync::Error::Trust(error)
        }
        RuntimeError::Management(ManagementFailure::WrongRole) => taypeer_sync::Error::Unauthorized,
        RuntimeError::Protocol
        | RuntimeError::TooLarge
        | RuntimeError::Profile(crate::profile::ProfileError::Invalid) => {
            taypeer_sync::Error::Protocol
        }
        RuntimeError::Profile(_) => taypeer_sync::Error::Storage(taypeer_storage::Error::Io),
        RuntimeError::Transport => taypeer_sync::Error::Transport,
        RuntimeError::Management(_)
        | RuntimeError::Closed
        | RuntimeError::SessionClosed(_)
        | RuntimeError::OperationInterrupted(_)
        | RuntimeError::ShutdownUnconfirmed
        | RuntimeError::RecoveryRequiresNewOperation => taypeer_sync::Error::State,
        RuntimeError::Service(_) => taypeer_sync::Error::State,
    }
}

#[cfg(test)]
mod tests;
