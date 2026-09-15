use crate::{AuthorKey, DeviceId, Digest, Error, Identity, PublicKey, Signature, TrustSetId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::DatabaseId;

const DOMAIN: &[u8] = b"taypeer/control/1";
const HANDOFF: &[u8] = b"taypeer/handoff-consent/1";
const MAX_CONTROLS: usize = 100_000;
const MAX_MEMBERS: usize = 1024;

/// A membership interval; re-admission starts a new interval even for the same key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Member {
    /// Bound author and transport identities.
    pub identity: Identity,
    /// Sequence at which this uninterrupted admission began.
    pub admitted_at: u64,
}

/// Public signed authority, independent of encrypted user-document conflicts.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Control {
    /// Control encoding version, independent of document and file versions.
    pub version: u16,
    /// Stable identity of the user data.
    pub database: DatabaseId,
    /// Independent admission lineage.
    pub trust_set: TrustSetId,
    /// Zero for genesis; increments exactly once per administrative transition.
    pub sequence: u64,
    /// Exact preceding signed control hash.
    pub previous: Option<Digest>,
    /// Read-key epoch; increases only with an independent new read key.
    pub epoch: u64,
    /// Exactly one managing member.
    pub manager: DeviceId,
    /// Current membership intervals.
    pub members: BTreeMap<DeviceId, Member>,
    /// Commitment to the encrypted database policy.
    pub policy: Digest,
    /// Logical schema supported by this control state.
    pub schema: u16,
    /// Durable retry identity of the transition, absent only in genesis.
    pub operation: Option<Digest>,
    /// Hash of the approved transition intent; passwords never appear here.
    pub intent: Option<Digest>,
    /// Recipient's consent is required for a change of manager.
    pub handoff: Option<HandoffConsent>,
}

/// A recipient's explicit consent to a particular connected handoff operation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct HandoffConsent {
    /// Control against which both devices approved the operation.
    pub previous: Digest,
    /// Recipient of management.
    pub recipient: DeviceId,
    /// Shared retry identity.
    pub operation: Digest,
    /// Recipient's author signature.
    pub signature: Signature,
}
impl HandoffConsent {
    /// The caller must gate this on an unlocked session and explicit approval.
    pub fn sign(chain: &ControlChain, key: &AuthorKey, operation: Digest) -> Result<Self, Error> {
        let recipient = key.device_id();
        if !chain.head().members.contains_key(&recipient) || recipient == chain.head().manager {
            return Err(Error::Unauthorized);
        }
        let previous = chain.head_hash()?;
        let signature = key.sign(HANDOFF, &(previous, recipient, operation))?;
        Ok(Self {
            previous,
            recipient,
            operation,
            signature,
        })
    }
    fn verify(&self, prior: &SignedControl) -> Result<(), Error> {
        if self.previous != prior.hash()? || self.recipient == prior.body.manager {
            return Err(Error::Stale);
        }
        let key = prior
            .body
            .members
            .get(&self.recipient)
            .ok_or(Error::Unauthorized)?
            .identity
            .author;
        key.verify(
            HANDOFF,
            &(self.previous, self.recipient, self.operation),
            &self.signature,
        )
    }
}

/// The author key of the preceding manager authenticates each successor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SignedControl {
    /// Canonical authority state.
    pub body: Control,
    /// Signature by the genesis manager or predecessor's manager.
    pub signature: Signature,
}
impl SignedControl {
    /// Stable identity includes both body and signature.
    pub fn hash(&self) -> Result<Digest, Error> {
        Digest::object(DOMAIN, self)
    }
    fn signed(body: Control, key: &AuthorKey) -> Result<Self, Error> {
        Ok(Self {
            signature: key.sign(DOMAIN, &body)?,
            body,
        })
    }
    fn verify(&self, key: PublicKey) -> Result<(), Error> {
        key.verify(DOMAIN, &self.body, &self.signature)
    }
}

/// Administrative intents are closed and independently checked on receipt.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum ControlTransition {
    /// Add an explicitly confirmed device with a fresh admission interval.
    Admit(Identity),
    /// Independent key rotation, optionally revoking one ordinary member.
    Rotate {
        /// Device excluded by this rotation, if any.
        revoke: Option<DeviceId>,
        /// Commitment to the newly protected policy.
        policy: Digest,
    },
    /// Change only shared policy without changing read-key authority.
    Policy(Digest),
    /// Hand management to an already admitted, consenting device.
    Transfer(HandoffConsent),
}

/// A validated chain rooted at an externally pinned genesis hash.
#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(transparent)]
pub struct ControlChain(Vec<SignedControl>);
impl ControlChain {
    /// Create the first authority state. This does not persist credentials or data.
    pub fn genesis(
        database: DatabaseId,
        identity: Identity,
        key: &AuthorKey,
        policy: Digest,
        schema: u16,
    ) -> Result<Self, Error> {
        identity.validate()?;
        if identity.author != key.public() || schema != 4 {
            return Err(Error::Unauthorized);
        }
        let manager = identity.device;
        let members = BTreeMap::from([(
            manager,
            Member {
                identity,
                admitted_at: 0,
            },
        )]);
        let body = Control {
            version: 1,
            database,
            trust_set: TrustSetId::random()?,
            sequence: 0,
            previous: None,
            epoch: 0,
            manager,
            members,
            policy,
            schema,
            operation: None,
            intent: None,
            handoff: None,
        };
        Ok(Self(vec![SignedControl::signed(body, key)?]))
    }
    /// Validate every transition against an independently obtained root.
    pub fn validate(records: Vec<SignedControl>, pinned_root: Digest) -> Result<Self, Error> {
        if records.is_empty() || records.len() > MAX_CONTROLS {
            return Err(Error::Limit);
        }
        if records[0].hash()? != pinned_root {
            return Err(Error::Unauthorized);
        }
        let first = &records[0];
        validate_body(&first.body)?;
        let c = &first.body;
        if c.sequence != 0
            || c.epoch != 0
            || c.previous.is_some()
            || c.members.len() != 1
            || c.operation.is_some()
            || c.intent.is_some()
            || c.handoff.is_some()
            || c.members.values().any(|m| m.admitted_at != 0)
        {
            return Err(Error::Invalid);
        }
        first.verify(c.members[&c.manager].identity.author)?;
        let mut operations = BTreeSet::new();
        for pair in records.windows(2) {
            validate_successor(&pair[0], &pair[1])?;
            if !operations.insert(pair[1].body.operation.ok_or(Error::Invalid)?) {
                return Err(Error::Invalid);
            }
        }
        Ok(Self(records))
    }
    /// Current accepted state; construction guarantees a nonempty chain.
    pub fn head(&self) -> &Control {
        &self.0[self.0.len() - 1].body
    }
    /// Signed records for bounded wire/storage encoding.
    pub fn records(&self) -> &[SignedControl] {
        &self.0
    }
    /// Pinned genesis fingerprint used for new peer enrollment.
    pub fn root(&self) -> Result<Digest, Error> {
        self.0[0].hash()
    }
    /// Identity of the current signed control.
    pub fn head_hash(&self) -> Result<Digest, Error> {
        self.0[self.0.len() - 1].hash()
    }
    /// Find the exact signed state associated with an incoming source.
    pub fn at(&self, hash: Digest) -> Result<&Control, Error> {
        self.0
            .iter()
            .find_map(|c| match c.hash() {
                Ok(id) if id == hash => Some(&c.body),
                _ => None,
            })
            .ok_or(Error::Stale)
    }
    /// Resolve an authenticated transport public key to a currently admitted device.
    pub fn admit_transport(&self, key: PublicKey) -> Result<DeviceId, Error> {
        self.head()
            .members
            .values()
            .find(|m| m.identity.transport == key)
            .map(|m| m.identity.device)
            .ok_or(Error::Unauthorized)
    }
    /// Whether an old source author remained continuously admitted through the head.
    pub fn continuous(&self, device: DeviceId, from: Digest) -> Result<bool, Error> {
        let old = self.at(from)?;
        let Some(member) = old.members.get(&device) else {
            return Ok(false);
        };
        Ok(self
            .head()
            .members
            .get(&device)
            .is_some_and(|current| current == member))
    }
    /// Extend or compare a verified chain. Incomparable signed states are a fork.
    /// The caller must durably retain both chains as evidence before resuming anything.
    pub fn reconcile(&self, incoming: &Self) -> Result<Self, Error> {
        if self.root()? != incoming.root()? {
            return Err(Error::Unauthorized);
        }
        if self.0.iter().zip(&incoming.0).any(|(a, b)| a != b) {
            return Err(Error::Fork);
        }
        Ok(if incoming.0.len() > self.0.len() {
            incoming.clone()
        } else {
            self.clone()
        })
    }
    /// Prepare a checked successor. Idempotent repeats return the existing chain;
    /// callers still have to durably commit it before reporting success.
    pub fn transition(
        &self,
        key: &AuthorKey,
        operation: Digest,
        intent: ControlTransition,
    ) -> Result<Self, Error> {
        let intent_hash = Digest::object(b"taypeer/control-intent/1", &intent)?;
        if let Some(prior) = self.0.iter().find(|c| c.body.operation == Some(operation)) {
            return if prior.body.intent == Some(intent_hash) {
                Ok(self.clone())
            } else {
                Err(Error::OperationMismatch)
            };
        }
        let head = self.head();
        if key.device_id() != head.manager {
            return Err(Error::Unauthorized);
        }
        let mut next = head.clone();
        next.previous = Some(self.head_hash()?);
        next.sequence = head.sequence.checked_add(1).ok_or(Error::Limit)?;
        next.operation = Some(operation);
        next.intent = Some(intent_hash);
        next.handoff = None;
        match intent {
            ControlTransition::Admit(identity) => {
                identity.validate()?;
                if next.members.contains_key(&identity.device) {
                    return Err(Error::Invalid);
                }
                next.members.insert(
                    identity.device,
                    Member {
                        identity,
                        admitted_at: next.sequence,
                    },
                );
            }
            ControlTransition::Rotate { revoke, policy } => {
                next.epoch = head.epoch.checked_add(1).ok_or(Error::Limit)?;
                next.policy = policy;
                if let Some(device) = revoke
                    && (device == head.manager || next.members.remove(&device).is_none())
                {
                    return Err(Error::Unauthorized);
                }
            }
            ControlTransition::Policy(policy) => next.policy = policy,
            ControlTransition::Transfer(consent) => {
                consent.verify(&self.0[self.0.len() - 1])?;
                if consent.operation != operation {
                    return Err(Error::OperationMismatch);
                }
                next.manager = consent.recipient;
                next.handoff = Some(consent);
            }
        }
        let mut records = self.0.clone();
        records.push(SignedControl::signed(next, key)?);
        Self::validate(records, self.root()?)
    }
}

fn validate_body(c: &Control) -> Result<(), Error> {
    if c.version != 1
        || c.schema != 4
        || c.members.is_empty()
        || c.members.len() > MAX_MEMBERS
        || !c.members.contains_key(&c.manager)
        || c.database.as_str().is_empty()
        || c.database.as_str().len() > 128
    {
        return Err(Error::Invalid);
    }
    let mut transports = BTreeSet::new();
    for (id, member) in &c.members {
        member.identity.validate()?;
        if *id != member.identity.device
            || member.admitted_at > c.sequence
            || !transports.insert(member.identity.transport)
        {
            return Err(Error::Invalid);
        }
    }
    Ok(())
}

fn validate_successor(prior: &SignedControl, next: &SignedControl) -> Result<(), Error> {
    let a = &prior.body;
    let b = &next.body;
    validate_body(b)?;
    next.verify(a.members[&a.manager].identity.author)?;
    if b.database != a.database
        || b.trust_set != a.trust_set
        || b.schema != a.schema
        || b.previous != Some(prior.hash()?)
        || Some(b.sequence) != a.sequence.checked_add(1)
        || b.operation.is_none()
        || b.intent.is_none()
    {
        return Err(Error::Invalid);
    }
    let added: Vec<_> = b
        .members
        .keys()
        .filter(|id| !a.members.contains_key(id))
        .collect();
    let removed: Vec<_> = a
        .members
        .keys()
        .filter(|id| !b.members.contains_key(id))
        .collect();
    if a.members
        .iter()
        .any(|(id, m)| b.members.get(id).is_some_and(|n| n != m))
    {
        return Err(Error::Invalid);
    }
    let inferred = if b.manager != a.manager {
        let consent = b.handoff.as_ref().ok_or(Error::Unauthorized)?;
        consent.verify(prior)?;
        if consent.recipient != b.manager
            || Some(consent.operation) != b.operation
            || b.members != a.members
            || b.epoch != a.epoch
            || b.policy != a.policy
        {
            return Err(Error::Invalid);
        }
        ControlTransition::Transfer(consent.clone())
    } else {
        if b.handoff.is_some() {
            return Err(Error::Invalid);
        }
        if b.epoch == a.epoch {
            if !removed.is_empty() || added.len() > 1 {
                return Err(Error::Unauthorized);
            }
            if let Some(id) = added.first() {
                let member = &b.members[id];
                if member.admitted_at != b.sequence || b.policy != a.policy {
                    return Err(Error::Invalid);
                }
                ControlTransition::Admit(member.identity)
            } else {
                ControlTransition::Policy(b.policy)
            }
        } else {
            if Some(b.epoch) != a.epoch.checked_add(1) || !added.is_empty() || removed.len() > 1 {
                return Err(Error::Invalid);
            }
            ControlTransition::Rotate {
                revoke: removed.first().map(|id| **id),
                policy: b.policy,
            }
        }
    };
    if b.intent != Some(Digest::object(b"taypeer/control-intent/1", &inferred)?) {
        return Err(Error::Invalid);
    }
    Ok(())
}
