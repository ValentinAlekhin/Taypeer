//! Device-local access marks. Only opaque identities, ordering and UTC are protected here.
use super::{NativeProfile, ProfileError, random_id};
use serde::{Deserialize, Serialize};
use std::sync::{Arc, Mutex, Weak};
use taypeer_core::{DatabaseId, EntryId};
use taypeer_trust::Digest;

/// UTC source for accepted secret actions. Unlike a merge clock, this may go backwards.
pub trait AccessClock: Send + Sync {
    /// Exact UTC milliseconds; failure must not invent a timestamp.
    fn utc_millis(&self) -> Result<i64, ProfileError>;
}
pub(crate) struct SystemAccessClock;
impl AccessClock for SystemAccessClock {
    fn utc_millis(&self) -> Result<i64, ProfileError> {
        match std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH) {
            Ok(elapsed) => i64::try_from(elapsed.as_millis()).map_err(|_| ProfileError::Io),
            Err(error) => i64::try_from(error.duration().as_millis())
                .ok()
                .and_then(i64::checked_neg)
                .ok_or(ProfileError::Io),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct AccessRecord {
    version: u16,
    database: DatabaseId,
    entry: EntryId,
    event: Digest,
    owner: Digest,
    accepted_order: AcceptedOrder,
    order: u64,
    utc_ms: i64,
}
impl AccessRecord {
    fn key(database: &DatabaseId, entry: &EntryId) -> String {
        let address = format!("{}:{database}{entry}", database.as_str().len());
        format!("entry-access:{}", Digest::of(address.as_bytes()))
    }
    fn key_for_self(&self) -> String {
        Self::key(&self.database, &self.entry)
    }
    fn validate(&self, database: &DatabaseId, entry: &EntryId) -> Result<(), ProfileError> {
        if self.version != 1
            || self.order == 0
            || self.accepted_order.0 == 0
            || &self.database != database
            || &self.entry != entry
        {
            return Err(ProfileError::Invalid);
        }
        Ok(())
    }
}
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
struct AcceptedOrder(u64);
struct AcceptedEvent {
    event: Digest,
    order: AcceptedOrder,
    utc_ms: i64,
    durable_order: Option<u64>,
}
struct AccessState {
    next_accepted_order: u64,
    next_order: u64,
    // One serialized owner reconciles an unknown publication before accepting another event.
    pending: Option<PendingAccess>,
}
#[derive(Clone)]
struct PendingAccess {
    profile: NativeProfile,
    record: AccessRecord,
}
pub(crate) struct EntryAccessStore {
    owner: Digest,
    clock: Arc<dyn AccessClock>,
    state: Mutex<AccessState>,
}
impl EntryAccessStore {
    pub(crate) fn new(clock: Arc<dyn AccessClock>) -> Result<Self, ProfileError> {
        Ok(Self {
            owner: random_id()?,
            clock,
            state: Mutex::new(AccessState {
                next_order: 0,
                next_accepted_order: 0,
                pending: None,
            }),
        })
    }
    fn load(
        profile: &NativeProfile,
        database: &DatabaseId,
        entry: &EntryId,
    ) -> Result<Option<AccessRecord>, ProfileError> {
        let record: Option<AccessRecord> =
            profile.load_state(&AccessRecord::key(database, entry))?;
        if let Some(record) = &record {
            record.validate(database, entry)?;
        }
        Ok(record)
    }
    pub(crate) fn accessed_at(
        &self,
        profile: &NativeProfile,
        database: &DatabaseId,
        entry: &EntryId,
    ) -> Result<Option<i64>, ProfileError> {
        let state = self.state.lock().map_err(|_| ProfileError::Credentials)?;
        if state.pending.as_ref().is_some_and(|pending| {
            &pending.record.database == database && &pending.record.entry == entry
        }) {
            return Err(ProfileError::CommitUncertain);
        }
        Ok(Self::load(profile, database, entry)?.map(|record| record.utc_ms))
    }
    pub(crate) fn grant(
        self: &Arc<Self>,
        profile: NativeProfile,
        database: DatabaseId,
        entry: EntryId,
    ) -> SecretAccessGrant {
        SecretAccessGrant {
            owner: Arc::downgrade(self),
            profile,
            database,
            entry,
            event: Mutex::new(None),
        }
    }
    fn publish(
        &self,
        state: &mut AccessState,
        profile: &NativeProfile,
        record: &AccessRecord,
    ) -> Result<(), ProfileError> {
        match profile.save_state(&record.key_for_self(), record) {
            Ok(()) => Ok(()),
            Err(ProfileError::CommitUncertain) => {
                state.pending = Some(PendingAccess {
                    profile: profile.clone(),
                    record: record.clone(),
                });
                Err(ProfileError::CommitUncertain)
            }
            Err(error) => Err(error),
        }
    }
    fn reconcile(&self, state: &mut AccessState) -> Result<(), ProfileError> {
        let Some(pending) = state.pending.clone() else {
            return Ok(());
        };
        let current = Self::load(
            &pending.profile,
            &pending.record.database,
            &pending.record.entry,
        )?;
        let record = retained_record(current.as_ref(), &pending.record)?;
        self.publish(state, &pending.profile, record)?;
        state.pending = None;
        Ok(())
    }
    fn confirm(
        &self,
        profile: &NativeProfile,
        database: &DatabaseId,
        entry: &EntryId,
        frozen: &mut Option<AcceptedEvent>,
    ) -> Result<i64, ProfileError> {
        let mut state = self.state.lock().map_err(|_| ProfileError::Credentials)?;
        if frozen.is_none() {
            let accepted_order = state
                .next_accepted_order
                .checked_add(1)
                .ok_or(ProfileError::Invalid)?;
            let event = random_id()?;
            let utc_ms = self.clock.utc_millis()?;
            state.next_accepted_order = accepted_order;
            *frozen = Some(AcceptedEvent {
                event,
                order: AcceptedOrder(accepted_order),
                utc_ms,
                durable_order: None,
            });
        }
        self.reconcile(&mut state)?;
        let current = Self::load(profile, database, entry)?;
        let event = frozen.as_mut().ok_or(ProfileError::Invalid)?;
        if let Some(record) = &current
            && record.owner == self.owner
            && record.accepted_order > event.order
        {
            // A newer accepted action can supersede an older event whose first
            // read failed before a durable ordinal could be resolved.
            self.publish(&mut state, profile, record)?;
            return Ok(record.utc_ms);
        }
        let order = match event.durable_order {
            Some(order) => order,
            None => {
                let order = state
                    .next_order
                    .max(current.as_ref().map_or(0, |record| record.order))
                    .checked_add(1)
                    .ok_or(ProfileError::Invalid)?;
                state.next_order = order;
                event.durable_order = Some(order);
                order
            }
        };
        let candidate = AccessRecord {
            version: 1,
            database: database.clone(),
            entry: entry.clone(),
            event: event.event,
            owner: self.owner,
            accepted_order: event.order,
            order,
            utc_ms: event.utc_ms,
        };
        let record = retained_record(current.as_ref(), &candidate)?;
        // Even an exact read-back is re-published: cache/read equality cannot confirm durability.
        self.publish(&mut state, profile, record)?;
        Ok(record.utc_ms)
    }
}
fn retained_record<'a>(
    current: Option<&'a AccessRecord>,
    event: &'a AccessRecord,
) -> Result<&'a AccessRecord, ProfileError> {
    match current {
        Some(record) if record.order > event.order => Ok(record),
        Some(record) if record.order == event.order && record != event => {
            Err(ProfileError::Invalid)
        }
        _ => Ok(event),
    }
}

/// An opaque permission to record one already accepted action on a saved entry.
/// It owns no secret, accepts no arbitrary target/time and survives document locking.
/// The first attempt freezes UTC and order; every retry reconciles the protected store
/// before acknowledging, preserving any newer event even when UTC went backwards.
pub struct SecretAccessGrant {
    owner: Weak<EntryAccessStore>,
    profile: NativeProfile,
    database: DatabaseId,
    entry: EntryId,
    event: Mutex<Option<AcceptedEvent>>,
}
impl SecretAccessGrant {
    /// Confirm only after the value was accepted for display or clipboard publication.
    /// Return the latest durably acknowledged mark, which may belong to a newer action.
    /// Failure affects this local metadata only; never repeat the secret action to retry.
    pub fn confirm(&self) -> Result<i64, ProfileError> {
        let mut event = self.event.lock().map_err(|_| ProfileError::Credentials)?;
        self.owner
            .upgrade()
            .ok_or(ProfileError::Credentials)?
            .confirm(&self.profile, &self.database, &self.entry, &mut event)
    }
}
impl std::fmt::Debug for SecretAccessGrant {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("SecretAccessGrant([opaque])")
    }
}

#[cfg(test)]
mod tests;
