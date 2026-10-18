//! Finite explicit actions. No gate survives a worker request or bounded network exchange.
use super::*;
use rand_core::{OsRng, RngCore};

fn challenge() -> Result<Digest, RuntimeError> {
    let mut bytes = [0; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| RuntimeError::Transport)?;
    Ok(Digest::from_bytes(bytes))
}
impl RuntimeHost {
    /// Query this operation's peer over a fresh authenticated bounded RPC. No saved
    /// exchange label, route or previously returned Ready value authorizes a later action.
    pub fn management_status(
        &self,
        intent: &ManagementIntent,
    ) -> Result<ManagementPeerStatus, RuntimeError> {
        let context = self.context.for_database(&intent.database)?;
        let snapshot = Owner::snapshot(&context, intent)?;
        let device = snapshot
            .chain()
            .admit_transport(context.transport.public())
            .map_err(|_| failure(ManagementFailure::WrongRole))?;
        let direction = if device == intent.manager {
            Direction::Source
        } else if device == intent.recipient {
            Direction::Recipient
        } else {
            return Err(failure(ManagementFailure::WrongRole));
        };
        let reply = self.management_request(&context, intent, direction, false)?;
        let phase = match reply.decision {
            ManagementDecision::Unknown => None,
            ManagementDecision::Proposed => Some(ManagementPhase::Proposed),
            ManagementDecision::Waiting => Some(ManagementPhase::WaitingDecision),
            ManagementDecision::Accepted(_) => Some(ManagementPhase::ConsentRecorded),
            ManagementDecision::Rejected => Some(ManagementPhase::Rejected),
            ManagementDecision::Committed(_) => Some(ManagementPhase::SourceCommitted),
            ManagementDecision::Applied(_) => Some(ManagementPhase::Completed),
        };
        Ok(ManagementPeerStatus {
            intent: reply.intent,
            phase,
            readiness: reply.readiness,
        })
    }

    /// Last confirmed protected handoff receipts for the selected working copy.
    /// This does not perform networking, acquire author credentials or unlock a process.
    pub fn management_handoffs(
        &self,
        database: &DatabaseId,
    ) -> Result<Vec<ManagementHandoff>, RuntimeError> {
        let context = self.context.for_database(database)?;
        let registration = Owner::registration(&context, database)?;
        Ok(context
            .management
            .store
            .records(&context.profile, &registration)?
            .iter()
            .map(Record::view)
            .collect())
    }

    /// Durably propose one explicit successor using an actual OPEN managing worker.
    /// Retry retains the original operation/head; a changed recipient refuses without mutation.
    pub fn propose_management(
        &self,
        session: &ManagementSession,
        recipient: DeviceId,
        operation: Digest,
    ) -> Result<ManagementHandoff, RuntimeError> {
        let context = self.context.for_database(&session.database)?;
        let registration = Owner::registration(&context, &session.database)?;
        let records = context
            .management
            .store
            .records(&context.profile, &registration)?;
        if let Some(record) = records
            .iter()
            .find(|record| record.intent.operation == operation)
        {
            if record.direction != Direction::Source || record.intent.recipient != recipient {
                return Err(failure(ManagementFailure::Conflict));
            }
            context
                .management
                .live(&session.database, Some(session.generation))?;
            return self.retry_management(&record.intent);
        }
        let live = context
            .management
            .live(&session.database, Some(session.generation))?;
        if live.basis.manager != live.basis.identity.device
            || live.basis.identity.transport != context.transport.public()
        {
            return Err(failure(ManagementFailure::WrongRole));
        }
        let intent = ManagementIntent {
            database: session.database.clone(),
            root: live.basis.root,
            previous: live.basis.control,
            operation,
            manager: live.basis.manager,
            recipient,
        };
        Owner::snapshot(&context, &intent)?;
        require_previous(&live, &intent, Direction::Source)?;
        context
            .management
            .store
            .update(&context.profile, &registration, &intent, |existing| {
                if existing.is_some() {
                    return Err(failure(ManagementFailure::Conflict));
                }
                Ok(Record {
                    intent: intent.clone(),
                    direction: Direction::Source,
                    phase: Phase::Proposed,
                })
            })?;
        live.client.control.check(false)?;
        self.retry_management(&intent)
    }

    /// Explicitly accept the exact saved proposal. The decision is retained before
    /// signing, and Accepted becomes visible only after protected consent publication ACK.
    pub fn accept_management(
        &self,
        session: &ManagementSession,
        intent: &ManagementIntent,
    ) -> Result<ManagementHandoff, RuntimeError> {
        if session.database != intent.database {
            return Err(failure(ManagementFailure::WrongRole));
        }
        self.accept_management_exact(session, intent, true)
    }
    fn accept_management_exact(
        &self,
        session: &ManagementSession,
        intent: &ManagementIntent,
        explicit: bool,
    ) -> Result<ManagementHandoff, RuntimeError> {
        let context = self.context.for_database(&intent.database)?;
        Owner::snapshot(&context, intent)?;
        let registration = Owner::registration(&context, &intent.database)?;
        let record = context
            .management
            .store
            .record(&context.profile, &registration, intent, true)?
            .ok_or_else(|| failure(ManagementFailure::Unknown))?;
        if record.direction != Direction::Recipient {
            return Err(failure(ManagementFailure::WrongRole));
        }
        match &record.phase {
            Phase::ConsentRecorded(_) | Phase::Completed(_) => {
                context
                    .management
                    .live(&intent.database, Some(session.generation))?;
                return Ok(record.view());
            }
            Phase::WaitingDecision if explicit => {}
            Phase::AcceptRequested => {}
            _ => return Err(failure(ManagementFailure::Conflict)),
        }
        let live = context
            .management
            .live(&intent.database, Some(session.generation))?;
        require_previous(&live, intent, Direction::Recipient)?;
        context
            .management
            .store
            .update(&context.profile, &registration, intent, |existing| {
                let mut record = existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                match record.phase {
                    Phase::WaitingDecision if explicit => record.phase = Phase::AcceptRequested,
                    Phase::AcceptRequested => {}
                    _ => return Err(failure(ManagementFailure::Conflict)),
                }
                Ok(record)
            })?;
        live.client.control.check(false)?;
        // Signing follows a fresh authenticated source check; its saved proposal,
        // rather than mere liveness, proves the source's explicit intent.
        let remote = self.management_request(&context, intent, Direction::Recipient, false)?;
        if remote.decision != ManagementDecision::Proposed {
            return Err(failure(ManagementFailure::Stale));
        }
        require_remote(&context, intent, Direction::Recipient, &remote.readiness)?;
        let live = context
            .management
            .live(&intent.database, Some(session.generation))?;
        require_previous(&live, intent, Direction::Recipient)?;
        let consent: taypeer_trust::HandoffConsent = serde_json::from_value(
            live.client
                .request(&Command::ConsentManagement(intent.operation), false)?,
        )
        .map_err(|_| RuntimeError::Protocol)?;
        if consent.previous != intent.previous
            || consent.operation != intent.operation
            || consent.recipient != intent.recipient
        {
            return Err(RuntimeError::Protocol);
        }
        let confirmed = context
            .management
            .live(&intent.database, Some(session.generation))?;
        require_previous(&confirmed, intent, Direction::Recipient)?;
        let record = context.management.store.update(
            &context.profile,
            &registration,
            intent,
            |existing| {
                let mut record = existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                match &record.phase {
                    Phase::AcceptRequested => record.phase = Phase::ConsentRecorded(consent),
                    Phase::ConsentRecorded(_) => {}
                    _ => return Err(failure(ManagementFailure::Conflict)),
                }
                Ok(record)
            },
        )?;
        live.client.control.check(false)?;
        Ok(record.view())
    }

    /// Durably reject only a proposal without an accepted/uncertain decision.
    /// Equal refusal retries return the same receipt; lock still revokes late UI output.
    pub fn reject_management(
        &self,
        session: &ManagementSession,
        intent: &ManagementIntent,
    ) -> Result<ManagementHandoff, RuntimeError> {
        if session.database != intent.database {
            return Err(failure(ManagementFailure::WrongRole));
        }
        let context = self.context.for_database(&intent.database)?;
        Owner::snapshot(&context, intent)?;
        let live = context
            .management
            .live(&intent.database, Some(session.generation))?;
        require_previous(&live, intent, Direction::Recipient)?;
        let registration = Owner::registration(&context, &intent.database)?;
        let record = context.management.store.update(
            &context.profile,
            &registration,
            intent,
            |existing| {
                let mut record = existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                if record.direction != Direction::Recipient {
                    return Err(failure(ManagementFailure::WrongRole));
                }
                match record.phase {
                    Phase::WaitingDecision => record.phase = Phase::Rejected,
                    Phase::Rejected => {}
                    _ => return Err(failure(ManagementFailure::Conflict)),
                }
                Ok(record)
            },
        )?;
        live.client.control.check(false)?;
        Ok(record.view())
    }

    /// Continue the original exact operation. Committed delivery does not require
    /// the source's former managing role or auto-unlock; precommit still requires OPEN.
    pub fn retry_management(
        &self,
        intent: &ManagementIntent,
    ) -> Result<ManagementHandoff, RuntimeError> {
        let context = self.context.for_database(&intent.database)?;
        let snapshot = Owner::snapshot(&context, intent)?;
        let registration = Owner::registration(&context, &intent.database)?;
        let record = context
            .management
            .store
            .record(&context.profile, &registration, intent, true)?
            .ok_or_else(|| failure(ManagementFailure::Unknown))?;
        if record.direction == Direction::Recipient {
            if record.phase == Phase::AcceptRequested {
                let live = context.management.live(&intent.database, None)?;
                return self.accept_management_exact(
                    &ManagementSession {
                        database: intent.database.clone(),
                        generation: live.generation,
                    },
                    intent,
                    false,
                );
            }
            context.management.reply(
                &context,
                intent.clone(),
                challenge()?,
                Direction::Recipient,
            )?;
            return context
                .management
                .store
                .record(&context.profile, &registration, intent, false)?
                .map(|record| record.view())
                .ok_or_else(|| failure(ManagementFailure::Unknown));
        }
        if matches!(&record.phase, Phase::Rejected | Phase::Completed(_)) {
            return Ok(record.view());
        }
        let committed = intent
            .committed_control(snapshot.chain())
            .map_err(sync_error)?;
        let transition = if record.phase != Phase::Proposed {
            committed.ok_or_else(|| failure(ManagementFailure::Stale))?
        } else {
            let client = context.management.client(&intent.database, None)?;
            if let Some(control) = committed {
                let consent = snapshot
                    .chain()
                    .records()
                    .iter()
                    .find(|record| record.hash().is_ok_and(|hash| hash == control))
                    .and_then(|record| record.body.handoff.clone())
                    .ok_or(RuntimeError::Protocol)?;
                // A visible control may belong to a lost/uncertain publication.
                // Only the worker's exact retained commit confirmation advances our phase.
                client.request(&Command::TransferManagement(consent), false)?;
                control
            } else {
                let live = match context.management.live(&intent.database, None) {
                    Ok(live) => {
                        require_previous(&live, intent, Direction::Source)?;
                        Some(live)
                    }
                    Err(RuntimeError::Service(taypeer_services::ServiceError::Storage(
                        taypeer_storage::Error::CommitUncertain,
                    ))) => None,
                    Err(error) => return Err(error),
                };
                let remote =
                    self.management_request(&context, intent, Direction::Source, live.is_some())?;
                match remote.decision {
                    ManagementDecision::Waiting | ManagementDecision::Unknown => {
                        return Ok(record.view());
                    }
                    ManagementDecision::Rejected => {
                        return context
                            .management
                            .store
                            .update(&context.profile, &registration, intent, |existing| {
                                let mut record =
                                    existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                                if record.phase != Phase::Proposed {
                                    return Err(failure(ManagementFailure::Conflict));
                                }
                                record.phase = Phase::Rejected;
                                Ok(record)
                            })
                            .map(|record| record.view());
                    }
                    ManagementDecision::Accepted(consent) => {
                        require_remote(&context, intent, Direction::Source, &remote.readiness)?;
                        if let Some(live) = &live {
                            require_previous(live, intent, Direction::Source)?;
                        }
                        client.control.check(false)?;
                        if consent.previous != intent.previous
                            || consent.operation != intent.operation
                            || consent.recipient != intent.recipient
                        {
                            return Err(RuntimeError::Protocol);
                        }
                        client.request(&Command::TransferManagement(consent), false)?;
                        let current = Owner::snapshot(&context, intent)?;
                        intent
                            .committed_control(current.chain())
                            .map_err(sync_error)?
                            .ok_or(RuntimeError::Protocol)?
                    }
                    ManagementDecision::Applied(_)
                    | ManagementDecision::Committed(_)
                    | ManagementDecision::Proposed => return Err(RuntimeError::Protocol),
                }
            }
        };
        let record = context.management.store.update(
            &context.profile,
            &registration,
            intent,
            |existing| {
                let mut record = existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                match &record.phase {
                    Phase::Proposed => record.phase = Phase::SourceCommitted(transition),
                    Phase::SourceCommitted(saved) | Phase::AwaitingApply(saved)
                        if *saved == transition => {}
                    Phase::Completed(_) => {}
                    _ => return Err(failure(ManagementFailure::Conflict)),
                }
                Ok(record)
            },
        )?;
        if record.view().phase == ManagementPhase::Completed {
            return Ok(record.view());
        }
        let address = management_address(&context, intent, Direction::Source)?;
        // Ordinary ciphertext exchange ACKs receipt, never application.
        self.exchange(address, intent.database.clone())?;
        context
            .management
            .store
            .update(&context.profile, &registration, intent, |existing| {
                let mut record = existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                if matches!(&record.phase, Phase::SourceCommitted(saved) if *saved == transition) {
                    record.phase = Phase::AwaitingApply(transition);
                }
                Ok(record)
            })?;
        let remote = self.management_request(&context, intent, Direction::Source, false)?;
        let ManagementDecision::Applied(applied) = remote.decision else {
            return context
                .management
                .store
                .record(&context.profile, &registration, intent, false)?
                .map(|record| record.view())
                .ok_or_else(|| failure(ManagementFailure::Unknown));
        };
        let current = Owner::snapshot(&context, intent)?;
        if applied.intent != *intent
            || applied.transition != transition
            || !applied_contains(current.chain(), transition, applied.control)?
        {
            return Err(RuntimeError::Protocol);
        }
        context
            .management
            .store
            .update(&context.profile, &registration, intent, |existing| {
                let mut record = existing.ok_or_else(|| failure(ManagementFailure::Unknown))?;
                match &record.phase {
                    Phase::SourceCommitted(saved) | Phase::AwaitingApply(saved)
                        if *saved == transition =>
                    {
                        record.phase = Phase::Completed(applied)
                    }
                    Phase::Completed(_) => {}
                    _ => return Err(failure(ManagementFailure::Conflict)),
                }
                Ok(record)
            })
            .map(|record| record.view())
    }

    fn management_request(
        &self,
        context: &HostContext,
        intent: &ManagementIntent,
        direction: Direction,
        proposal: bool,
    ) -> Result<ManagementReply, RuntimeError> {
        let address = management_address(context, intent, direction)?;
        let node = self.node_for(&intent.database)?;
        let challenge = challenge()?;
        let command = if proposal {
            taypeer_sync::Command::ManagementProposal {
                intent: intent.clone(),
                challenge,
                address: node.address(),
            }
        } else {
            taypeer_sync::Command::ManagementStatus {
                intent: intent.clone(),
                challenge,
            }
        };
        let reply = self
            .runtime
            .block_on(node.request(address, command))
            .map_err(|error| {
                if error == taypeer_sync::Error::State {
                    failure(ManagementFailure::Unsupported)
                } else {
                    sync_error(error)
                }
            })?;
        let taypeer_sync::Reply::Management(reply) = reply else {
            return Err(failure(ManagementFailure::Unsupported));
        };
        if reply.intent != *intent || reply.challenge != challenge {
            return Err(RuntimeError::Protocol);
        }
        Ok(*reply)
    }
}

fn management_address(
    context: &HostContext,
    intent: &ManagementIntent,
    direction: Direction,
) -> Result<taypeer_sync::EndpointAddr, RuntimeError> {
    let snapshot = Owner::snapshot(context, intent)?;
    let previous = snapshot
        .chain()
        .at(intent.previous)
        .map_err(|_| failure(ManagementFailure::Stale))?;
    let remote = match direction {
        Direction::Source => intent.recipient,
        Direction::Recipient => intent.manager,
    };
    let peer = previous
        .members
        .get(&remote)
        .ok_or_else(|| failure(ManagementFailure::WrongRole))?
        .identity
        .transport;
    let address = context
        .coordinator
        .routes()
        .map_err(sync_error)?
        .get(&peer)
        .cloned()
        .ok_or(RuntimeError::Transport)?;
    if *address.id.as_bytes() != *peer.as_bytes() {
        return Err(RuntimeError::Protocol);
    }
    Ok(address)
}
fn require_remote(
    context: &HostContext,
    intent: &ManagementIntent,
    direction: Direction,
    readiness: &ManagementReadiness,
) -> Result<(), RuntimeError> {
    let ManagementReadiness::Ready {
        identity,
        control,
        generation,
        ..
    } = readiness
    else {
        return Err(failure(match readiness {
            ManagementReadiness::Locked => ManagementFailure::NeedsUnlock,
            ManagementReadiness::PendingApply => ManagementFailure::PendingApply,
            ManagementReadiness::Ready { .. } => unreachable!(),
        }));
    };
    let snapshot = Owner::snapshot(context, intent)?;
    let previous = snapshot
        .chain()
        .at(intent.previous)
        .map_err(|_| failure(ManagementFailure::Stale))?;
    let device = match direction {
        Direction::Source => intent.recipient,
        Direction::Recipient => intent.manager,
    };
    if *generation == 0
        || *control != intent.previous
        || identity.device != device
        || previous
            .members
            .get(&device)
            .is_none_or(|member| member.identity != *identity)
    {
        return Err(failure(ManagementFailure::Stale));
    }
    Ok(())
}
