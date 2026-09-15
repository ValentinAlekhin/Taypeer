//! Manager operations prepare one signed control and its encrypted accepted baseline.
use super::*;
use taypeer_storage::{ArchiveJournal, InvitationRecord};
use taypeer_trust::{
    ControlTransition, DeviceId, HandoffConsent, Invitation, InvitationSecret, InvitationStatus,
};

pub(super) struct Transition {
    pub chain: ControlChain,
    pub header: Vec<u8>,
    pub journal: ArchiveJournal,
}
impl ManagedState {
    fn manager(&self) -> Result<&AuthorKey, ServiceError> {
        let key = self.writer()?;
        if key.device_id() != self.snapshot.chain().head().manager {
            return Err(ServiceError::Unauthorized);
        }
        Ok(key)
    }
    fn journal(&mut self, journal: ArchiveJournal) -> Result<(), ServiceError> {
        self.manager()?;
        let body = &self.snapshot.metadata().manifest.body;
        let snapshot = self.port.commit(PreparedCommit {
            expected: self.snapshot.fingerprint(),
            control: self.snapshot.chain().head_hash()?,
            controls: self.snapshot.chain().records().to_vec(),
            objects: Vec::new(),
            remove: BTreeSet::new(),
            checkpoint: body.checkpoint,
            baseline: body.baseline,
            journal,
        })?;
        self.snapshot = snapshot;
        Ok(())
    }
}
impl DatabaseService {
    /// Public verified authority and portable workflow state, without any credential or bearer secret.
    pub fn authority(
        &self,
        session: &SessionToken,
    ) -> Result<(ControlChain, ArchiveJournal), ServiceError> {
        let managed = self
            .checked(session)?
            .managed
            .as_ref()
            .ok_or(ServiceError::InvalidContext)?;
        let snapshot = managed.port.snapshot()?;
        Ok((
            snapshot.chain().clone(),
            snapshot.metadata().journal.clone(),
        ))
    }
    /// Durably issue a five-minute invitation before returning its explicitly revealed bearer code.
    pub fn create_invitation(
        &mut self,
        session: &SessionToken,
        now_seconds: u64,
    ) -> Result<(Invitation, InvitationSecret), ServiceError> {
        let state = self.checked_mut(session)?;
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let secret = InvitationSecret::generate()?;
        let invitation = Invitation::create(
            managed.snapshot.chain(),
            managed.manager()?,
            &secret,
            now_seconds,
        )?;
        let mut journal = managed.snapshot.metadata().journal.clone();
        journal.invitations.insert(
            invitation.id()?,
            InvitationRecord {
                invitation: invitation.clone(),
                status: InvitationStatus::Available,
            },
        );
        managed.journal(journal)?;
        Ok((invitation, secret))
    }
    /// Approve the recipient whose proof is already durable. Equal retries reuse the admission.
    pub fn approve_invitation(
        &mut self,
        session: &SessionToken,
        request: Digest,
        now_seconds: u64,
    ) -> Result<DeviceId, ServiceError> {
        let state = self.checked_mut(session)?;
        let document = state.document().clone();
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let current = managed.port.snapshot()?;
        if current.chain() != managed.snapshot.chain() {
            return Err(ServiceError::ExpiredSession);
        }
        managed.snapshot = current;
        let author = managed.manager()?;
        let mut journal = managed.snapshot.metadata().journal.clone();
        let record = journal
            .invitations
            .get_mut(&request)
            .ok_or(ServiceError::NotFound)?;
        if let InvitationStatus::Accepted { recipient, .. } = record.status {
            return Ok(recipient);
        }
        record
            .invitation
            .verify(managed.snapshot.chain(), now_seconds)?;
        let InvitationStatus::Requested(proof) = &record.status else {
            return Err(taypeer_trust::Error::InvitationUnavailable.into());
        };
        let recipient = proof.recipient;
        proof.verify(&record.invitation, recipient.transport)?;
        let chain = managed.snapshot.chain().transition(
            author,
            request,
            ControlTransition::Admit(recipient),
        )?;
        record.status = InvitationStatus::Accepted {
            recipient: recipient.device,
            control: chain.head_hash()?,
        };
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        let transition = Transition {
            chain,
            header: open.header.clone(),
            journal,
        };
        managed.persist(
            &document,
            open.metadata.clone(),
            Vec::new(),
            Some(transition),
        )?;
        Ok(recipient.device)
    }
    /// Reject or cancel a pending invitation without reusing its bearer token.
    pub fn close_invitation(
        &mut self,
        session: &SessionToken,
        request: Digest,
        reject: bool,
    ) -> Result<(), ServiceError> {
        let state = self.checked_mut(session)?;
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let current = managed.port.snapshot()?;
        if current.chain() != managed.snapshot.chain() {
            return Err(ServiceError::ExpiredSession);
        }
        managed.snapshot = current;
        let mut journal = managed.snapshot.metadata().journal.clone();
        let record = journal
            .invitations
            .get_mut(&request)
            .ok_or(ServiceError::NotFound)?;
        if matches!(record.status, InvitationStatus::Accepted { .. }) {
            return Err(taypeer_trust::Error::InvitationUnavailable.into());
        }
        record.status = if reject {
            InvitationStatus::Rejected
        } else {
            InvitationStatus::Cancelled
        };
        managed.journal(journal)
    }
    /// Create recipient consent, bound to this exact authority head and operation identity.
    pub fn consent_management(
        &self,
        session: &SessionToken,
        operation: Digest,
    ) -> Result<HandoffConsent, ServiceError> {
        let managed = self
            .checked(session)?
            .managed
            .as_ref()
            .ok_or(ServiceError::InvalidContext)?;
        Ok(HandoffConsent::sign(
            managed.snapshot.chain(),
            managed.writer()?,
            operation,
        )?)
    }
    /// Relinquish management durably before a successor is notified. Retry uses the same consent.
    pub fn transfer_management(
        &mut self,
        session: &SessionToken,
        consent: HandoffConsent,
    ) -> Result<(), ServiceError> {
        let state = self.checked_mut(session)?;
        let document = state.document().clone();
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let chain = managed.snapshot.chain().transition(
            managed.writer()?,
            consent.operation,
            ControlTransition::Transfer(consent),
        )?;
        if chain == *managed.snapshot.chain() {
            return Ok(());
        }
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        let transition = Transition {
            chain,
            header: open.header.clone(),
            journal: managed.snapshot.metadata().journal.clone(),
        };
        managed.persist(
            &document,
            open.metadata.clone(),
            Vec::new(),
            Some(transition),
        )
    }
    /// Rotate to an independent key and a different password, optionally revoking one device.
    /// The old portable file is durably preserved first; historical keys remain encrypted under the new key.
    pub fn rotate_password(
        &mut self,
        session: &SessionToken,
        operation: Digest,
        password: &[u8],
        revoke: Option<DeviceId>,
    ) -> Result<(), ServiceError> {
        let state = self.checked_mut(session)?;
        let document = state.document().clone();
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        managed.manager()?;
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        let mut metadata = open.metadata.clone();
        let intent = codec::AdministrativeIntent::Rotation {
            revoke,
            password: Digest::object(
                b"taypeer/private-rotation-intent/1",
                &(metadata.policy_salt.as_ref(), password),
            )?,
        };
        if metadata.administrative_retry(operation, &intent)? {
            return Ok(());
        }
        let chain = managed.snapshot.chain().transition(
            managed.manager()?,
            operation,
            ControlTransition::Rotate {
                revoke,
                policy: metadata.commitment()?,
            },
        )?;
        if chain == *managed.snapshot.chain() {
            return Ok(());
        }
        let checkpoint = managed
            .snapshot
            .object(managed.snapshot.metadata().manifest.body.checkpoint)?;
        if checkpoint.unlock_key(password).is_ok() {
            return Err(ServiceError::InvalidInput);
        }
        managed.port.preserve_before(operation)?;
        let (header, key) =
            taypeer_storage::create_epoch(password, metadata.policy.kdf_target_ms())?;
        metadata
            .keys
            .insert(chain.head().epoch, Zeroizing::new(*key.secret_bytes()));
        metadata.administration.insert(operation, intent);
        let transition = Transition {
            chain,
            header,
            journal: managed.snapshot.metadata().journal.clone(),
        };
        managed.persist(&document, metadata, Vec::new(), Some(transition))
    }
    /// Update shared quotas. Changing the KDF target additionally authenticates the
    /// current password and creates a new independent epoch with the requested calibration.
    pub fn set_database_policy(
        &mut self,
        session: &SessionToken,
        operation: Digest,
        policy: DatabasePolicy,
        password: Option<&[u8]>,
    ) -> Result<(), ServiceError> {
        let state = self.checked_mut(session)?;
        let document = state.document().clone();
        let managed = state.managed.as_mut().ok_or(ServiceError::InvalidContext)?;
        let author = managed.manager()?;
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        let receipt = codec::AdministrativeIntent::Policy(policy);
        if open.metadata.administrative_retry(operation, &receipt)? {
            return Ok(());
        }
        let kdf_changed = policy.kdf_target_ms() != open.metadata.policy.kdf_target_ms();
        let mut metadata = open.metadata.clone();
        metadata.policy = policy;
        let intent = if kdf_changed {
            ControlTransition::Rotate {
                revoke: None,
                policy: metadata.commitment()?,
            }
        } else {
            ControlTransition::Policy(metadata.commitment()?)
        };
        let chain = managed
            .snapshot
            .chain()
            .transition(author, operation, intent)?;
        if chain == *managed.snapshot.chain() {
            return Ok(());
        }
        let header = if kdf_changed {
            let password = password.ok_or(ServiceError::InvalidInput)?;
            let checkpoint = managed
                .snapshot
                .object(managed.snapshot.metadata().manifest.body.checkpoint)?;
            checkpoint.unlock_key(password)?;
            managed.port.preserve_before(operation)?;
            let (header, key) = taypeer_storage::create_epoch(password, policy.kdf_target_ms())?;
            metadata
                .keys
                .insert(chain.head().epoch, Zeroizing::new(*key.secret_bytes()));
            header
        } else {
            open.header.clone()
        };
        metadata.administration.insert(operation, receipt);
        let transition = Transition {
            chain,
            header,
            journal: managed.snapshot.metadata().journal.clone(),
        };
        managed.persist(&document, metadata, Vec::new(), Some(transition))
    }
}
