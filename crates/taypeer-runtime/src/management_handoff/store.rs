//! Protected exact decisions. The short gate is never held through worker/network I/O.
use super::*;
use crate::profile::{NativeProfile, ProfileError, Registration};

const VERSION: u16 = 1;
const MAX_HANDOFFS: usize = 32;

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    pub intent: ManagementIntent,
    pub direction: Direction,
    pub phase: Phase,
}
#[derive(Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum Direction {
    Source,
    Recipient,
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "kind",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub(super) enum Phase {
    Proposed,
    WaitingDecision,
    // An explicit decision is retained before signing; it is not an Accepted ACK.
    AcceptRequested,
    ConsentRecorded(taypeer_trust::HandoffConsent),
    Rejected,
    SourceCommitted(Digest),
    AwaitingApply(Digest),
    Completed(ManagementApplied),
}
impl Record {
    pub fn view(&self) -> ManagementHandoff {
        let phase = match &self.phase {
            Phase::Proposed => ManagementPhase::Proposed,
            Phase::WaitingDecision => ManagementPhase::WaitingDecision,
            Phase::AcceptRequested => ManagementPhase::AcceptRequested,
            Phase::ConsentRecorded(_) => ManagementPhase::ConsentRecorded,
            Phase::Rejected => ManagementPhase::Rejected,
            Phase::SourceCommitted(_) => ManagementPhase::SourceCommitted,
            Phase::AwaitingApply(_) => ManagementPhase::AwaitingApply,
            Phase::Completed(_) => ManagementPhase::Completed,
        };
        ManagementHandoff {
            intent: self.intent.clone(),
            phase,
            applied: match &self.phase {
                Phase::Completed(applied) => Some(applied.clone()),
                _ => None,
            },
        }
    }
    pub fn decision(&self) -> ManagementDecision {
        match &self.phase {
            Phase::WaitingDecision | Phase::AcceptRequested => ManagementDecision::Waiting,
            Phase::ConsentRecorded(consent) => ManagementDecision::Accepted(consent.clone()),
            Phase::Rejected => ManagementDecision::Rejected,
            Phase::Completed(receipt) => ManagementDecision::Applied(receipt.clone()),
            Phase::Proposed => ManagementDecision::Proposed,
            Phase::SourceCommitted(control) | Phase::AwaitingApply(control) => {
                ManagementDecision::Committed(*control)
            }
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Journal {
    version: u16,
    database: DatabaseId,
    root: Digest,
    working_copy: Digest,
    records: Vec<Record>,
}
impl Journal {
    fn key(&self) -> String {
        format!("management:{}:{}", self.database, self.working_copy)
    }
    fn new(registration: &Registration) -> Self {
        Self {
            version: VERSION,
            database: registration.database.clone(),
            root: registration.root,
            working_copy: registration.working_copy,
            records: Vec::new(),
        }
    }
    fn validate(&self, registration: &Registration) -> Result<(), RuntimeError> {
        if self.version != VERSION
            || self.database != registration.database
            || self.root != registration.root
            || self.working_copy != registration.working_copy
            || self.records.len() > MAX_HANDOFFS
        {
            return Err(ProfileError::Invalid.into());
        }
        let mut operations = std::collections::BTreeSet::new();
        for record in &self.records {
            if record.intent.database != self.database
                || record.intent.root != self.root
                || record.intent.manager == record.intent.recipient
                || !operations.insert(record.intent.operation)
            {
                return Err(ProfileError::Invalid.into());
            }
            let valid = matches!(
                (&record.direction, &record.phase),
                (
                    Direction::Source,
                    Phase::Proposed
                        | Phase::Rejected
                        | Phase::SourceCommitted(_)
                        | Phase::AwaitingApply(_)
                        | Phase::Completed(_),
                ) | (
                    Direction::Recipient,
                    Phase::WaitingDecision
                        | Phase::AcceptRequested
                        | Phase::ConsentRecorded(_)
                        | Phase::Rejected
                        | Phase::Completed(_),
                )
            );
            if !valid {
                return Err(ProfileError::Invalid.into());
            }
            if let Phase::ConsentRecorded(consent) = &record.phase
                && (consent.previous != record.intent.previous
                    || consent.operation != record.intent.operation
                    || consent.recipient != record.intent.recipient)
            {
                return Err(ProfileError::Invalid.into());
            }
            if let Phase::Completed(applied) = &record.phase
                && applied.intent != record.intent
            {
                return Err(ProfileError::Invalid.into());
            }
        }
        Ok(())
    }
}

#[derive(Default)]
pub(super) struct Store {
    pending: Mutex<Option<PendingJournal>>,
}
struct PendingJournal {
    previous: Journal,
    candidate: Journal,
}
impl Store {
    fn publish(
        profile: &NativeProfile,
        pending: &mut Option<PendingJournal>,
        previous: &Journal,
        journal: &Journal,
    ) -> Result<(), RuntimeError> {
        match profile.save_state(&journal.key(), journal) {
            Ok(()) => Ok(()),
            Err(error) => {
                if error == ProfileError::CommitUncertain {
                    *pending = Some(PendingJournal {
                        previous: previous.clone(),
                        candidate: journal.clone(),
                    });
                }
                Err(error.into())
            }
        }
    }
    fn read_locked(
        profile: &NativeProfile,
        pending: &mut Option<PendingJournal>,
        registration: &Registration,
    ) -> Result<Journal, RuntimeError> {
        if let Some(publication) = pending.as_ref() {
            let candidate = &publication.candidate;
            let actual = profile.load_state::<Journal>(&candidate.key())?;
            if actual
                .as_ref()
                .is_none_or(|actual| actual != candidate && actual != &publication.previous)
                && !(actual.is_none() && publication.previous.records.is_empty())
            {
                return Err(ProfileError::Invalid.into());
            }
            // Even an equal read cannot clear an uncertain publication: republish to ACK.
            profile.save_state(&candidate.key(), candidate)?;
            *pending = None;
        }
        let key = Journal::new(registration).key();
        let journal = profile
            .load_state::<Journal>(&key)?
            .unwrap_or_else(|| Journal::new(registration));
        journal.validate(registration)?;
        Ok(journal)
    }
    pub fn records(
        &self,
        profile: &NativeProfile,
        registration: &Registration,
    ) -> Result<Vec<Record>, RuntimeError> {
        let mut pending = self.pending.lock().map_err(|_| RuntimeError::Transport)?;
        let journal = Self::read_locked(profile, &mut pending, registration)?;
        if !journal.records.is_empty() {
            // A cold owner has no in-memory publication outcome. Equal protected
            // bytes may follow a lost durability reply; only republishing confirms them.
            Self::publish(profile, &mut pending, &journal, &journal)?;
        }
        Ok(journal.records)
    }
    pub fn record(
        &self,
        profile: &NativeProfile,
        registration: &Registration,
        intent: &ManagementIntent,
        confirm: bool,
    ) -> Result<Option<Record>, RuntimeError> {
        let mut pending = self.pending.lock().map_err(|_| RuntimeError::Transport)?;
        let journal = Self::read_locked(profile, &mut pending, registration)?;
        let record = journal
            .records
            .iter()
            .find(|record| record.intent.operation == intent.operation)
            .cloned();
        if record
            .as_ref()
            .is_some_and(|record| record.intent != *intent)
        {
            return Err(failure(ManagementFailure::Conflict));
        }
        if confirm && record.is_some() {
            Self::publish(profile, &mut pending, &journal, &journal)?;
        }
        Ok(record)
    }
    pub fn update(
        &self,
        profile: &NativeProfile,
        registration: &Registration,
        intent: &ManagementIntent,
        update: impl FnOnce(Option<Record>) -> Result<Record, RuntimeError>,
    ) -> Result<Record, RuntimeError> {
        let mut pending = self.pending.lock().map_err(|_| RuntimeError::Transport)?;
        let mut journal = Self::read_locked(profile, &mut pending, registration)?;
        let previous = journal.clone();
        let position = journal
            .records
            .iter()
            .position(|record| record.intent.operation == intent.operation);
        let existing = position.map(|position| journal.records[position].clone());
        if existing
            .as_ref()
            .is_some_and(|record| record.intent != *intent)
        {
            return Err(failure(ManagementFailure::Conflict));
        }
        let record = update(existing)?;
        if record.intent != *intent {
            return Err(failure(ManagementFailure::Conflict));
        }
        match position {
            Some(position) => journal.records[position] = record.clone(),
            None if journal.records.len() < MAX_HANDOFFS => journal.records.push(record.clone()),
            None => return Err(failure(ManagementFailure::Capacity)),
        }
        journal.validate(registration)?;
        Self::publish(profile, &mut pending, &previous, &journal)?;
        Ok(record)
    }
}
