//! Typed projections of the common invitation, membership and exchange workflows.
use super::{CipherWriter, DocumentProcess, DocumentSession, failure, runtime, session::Launcher};
use crate::{AndroidError, CiphertextFiles, Host};
use std::sync::Arc;
use taypeer_runtime::{Command, InvitationCode, JoinProgress, NetworkCancellation};
use taypeer_trust::{Digest, InvitationStatus};
use zeroize::Zeroizing;

/// A locked enrollment result, never a claim that plaintext was applied.
#[derive(uniffi::Enum)]
pub enum EnrollmentProgress {
    /// Manager approval is outstanding and can be resumed after restart.
    Pending {
        /// Stable nonsecret request identity.
        request: String,
    },
    /// The internal ciphertext working copy and catalog are durable.
    Received {
        /// Logical database identity; still locked.
        database: String,
    },
    /// The manager rejected, cancelled or expired the request.
    Rejected,
}
impl From<JoinProgress> for EnrollmentProgress {
    fn from(progress: JoinProgress) -> Self {
        match progress {
            JoinProgress::Pending(id) => Self::Pending {
                request: id.to_string(),
            },
            JoinProgress::Received(id) => Self::Received {
                database: id.to_string(),
            },
            JoinProgress::Rejected => Self::Rejected,
        }
    }
}
/// Public durable intent, without its bearer code or keys.
#[derive(uniffi::Record)]
pub struct EnrollmentRow {
    /// Stable request identity.
    pub request: String,
    /// Logical database identity.
    pub database: String,
}
/// Exact durable invitation lifecycle.
#[derive(uniffi::Enum)]
pub enum InvitationPhase {
    /// Issued but not presented.
    Available,
    /// Recipient proof is awaiting explicit approval.
    Requested,
    /// Admission was durably committed.
    Accepted,
    /// Manager rejected the request.
    Rejected,
    /// Issuer cancelled the code.
    Cancelled,
}
/// Public request displayed for manager review.
#[derive(uniffi::Record)]
pub struct InvitationRow {
    /// Stable request identity.
    pub request: String,
    /// Exact durable workflow phase.
    pub phase: InvitationPhase,
    /// Verified recipient device, if presented.
    pub recipient: Option<String>,
}
/// Verified member and latest attempt, not a presence claim.
#[derive(uniffi::Record)]
pub struct DeviceRow {
    /// Stable author identity.
    pub id: String,
    /// This profile owns the device.
    pub local: bool,
    /// Current managing device.
    pub manager: bool,
    /// Last exchange succeeded, failed, or has not happened.
    pub last_exchange_succeeded: Option<bool>,
}
/// Content-free exchange projection available while locked.
#[derive(uniffi::Record)]
pub struct ExchangeView {
    /// Endpoint lifetime, not peer reachability.
    pub running: bool,
    /// Optional selected database.
    pub database: Option<String>,
    /// Verified member identities.
    pub devices: Vec<DeviceRow>,
    /// Durable admission workflows; bearer material is excluded.
    pub invitations: Vec<InvitationRow>,
}
#[uniffi::export]
impl Host {
    /// Redeem an explicitly entered bounded bearer code. A proof-only isolated
    /// generation exits before host networking; no master password is acquired.
    pub fn join_invitation(
        &self,
        code: String,
        process: Box<dyn DocumentProcess>,
        files: Box<dyn CiphertextFiles>,
    ) -> Result<EnrollmentProgress, AndroidError> {
        let code = Zeroizing::new(code);
        if code.len() > super::MAX_METADATA {
            return Err(AndroidError::InvalidOptions);
        }
        let code: InvitationCode =
            serde_json::from_str(&code).map_err(|_| AndroidError::InvalidOptions)?;
        let request = code.invitation.id().map_err(failure)?;
        let proof =
            if let Some(pending) = self.runtime.pending_joins().map_err(runtime)?.get(&request) {
                pending.proof.clone()
            } else {
                // This capability path never becomes a document. Keeping it distinct
                // from the received file allows exact completed-operation retries.
                let path = self
                    .runtime
                    .creation_path(&taypeer_core::OperationId::new(format!(
                        "join-proof:{request}"
                    )))
                    .map_err(runtime)?;
                let writer = Arc::new(CipherWriter {
                    writer: self
                        .runtime
                        .platform_cipher_writer(&path)
                        .map_err(runtime)?,
                    files: Arc::from(files),
                });
                self.runtime
                    .platform_join_proof(
                        &Launcher {
                            writer,
                            process: Arc::from(process),
                            selected: Arc::new(super::SelectedTransfersHost::default()),
                        },
                        code.invitation.clone(),
                    )
                    .map_err(runtime)?
            };
        self.runtime
            .join_platform_proof(code, proof, &NetworkCancellation::default())
            .map(Into::into)
            .map_err(runtime)
    }
    /// Resume a public durable proof without a bearer or an unlocked document.
    pub fn resume_join(&self, request: String) -> Result<EnrollmentProgress, AndroidError> {
        self.runtime
            .resume_join(super::digest(&request)?, String::new())
            .map(Into::into)
            .map_err(runtime)
    }
    /// List durable public requests, including requests from previous lifetimes.
    pub fn pending_joins(&self) -> Result<Vec<EnrollmentRow>, AndroidError> {
        self.runtime
            .pending_joins()
            .map(|rows| {
                rows.into_iter()
                    .map(|(id, pending)| EnrollmentRow {
                        request: id.to_string(),
                        database: pending.invitation.database.to_string(),
                    })
                    .collect()
            })
            .map_err(runtime)
    }
    /// Schedule the common automatic exchange, without creating duplicate jobs.
    pub fn wake_exchange(&self) -> Result<(), AndroidError> {
        self.runtime.wake_network().map_err(runtime)
    }
    /// Read verified public state even while the selected file is locked.
    pub fn exchange_view(&self, database: Option<String>) -> Result<ExchangeView, AndroidError> {
        let snapshot = self.runtime.network_snapshot().map_err(runtime)?;
        let selected = database.and_then(|id| {
            snapshot
                .databases
                .into_iter()
                .find(|db| db.database.as_str() == id)
        });
        let (database, devices, invitations) = match selected {
            Some(db) => (
                Some(db.database.to_string()),
                db.devices
                    .into_iter()
                    .map(|device| DeviceRow {
                        id: device.id.to_string(),
                        local: device.local,
                        manager: device.manager,
                        last_exchange_succeeded: device.progress.map(|p| p.result.is_ok()),
                    })
                    .collect(),
                db.invitations
                    .into_iter()
                    .map(|(request, state)| {
                        let (phase, recipient) = match state {
                            InvitationStatus::Available => (InvitationPhase::Available, None),
                            InvitationStatus::Requested(proof) => (
                                InvitationPhase::Requested,
                                Some(proof.recipient.device.to_string()),
                            ),
                            InvitationStatus::Accepted { recipient, .. } => {
                                (InvitationPhase::Accepted, Some(recipient.to_string()))
                            }
                            InvitationStatus::Rejected => (InvitationPhase::Rejected, None),
                            InvitationStatus::Cancelled => (InvitationPhase::Cancelled, None),
                        };
                        InvitationRow {
                            request: request.to_string(),
                            phase,
                            recipient,
                        }
                    })
                    .collect(),
            ),
            None => (None, Vec::new(), Vec::new()),
        };
        Ok(ExchangeView {
            running: snapshot.running,
            database,
            devices,
            invitations,
        })
    }
}

/// Manager-controlled attachment quotas and KDF target, validated in Rust.
#[derive(uniffi::Record)]
pub struct PolicyView {
    /// Per-attachment byte quota.
    pub attachment_bytes: u64,
    /// Total retained attachment byte quota.
    pub total_attachment_bytes: u64,
    /// Managing-device calibration target in milliseconds.
    pub kdf_target_ms: u32,
}
#[uniffi::export]
impl DocumentSession {
    /// Explicitly reveal one durably issued five-minute invitation code.
    pub fn create_invitation(&self) -> Result<String, AndroidError> {
        let (invitation, secret): (taypeer_trust::Invitation, [u8; 32]) =
            self.command(Command::CreateInvitation)?;
        let address = self
            .host
            .network_address_for(&invitation.database)
            .map_err(runtime)?;
        serde_json::to_string(&InvitationCode {
            invitation,
            secret: Zeroizing::new(secret),
            address,
        })
        .map_err(failure)
    }
    /// Approval is an explicit security action against the reviewed request.
    pub fn approve_invitation(&self, request: String) -> Result<(), AndroidError> {
        self.command::<serde_json::Value>(Command::ApproveInvitation(super::digest(&request)?))
            .map(|_| ())
    }
    /// Explicit rejection or cancellation; no unused code is silently accepted.
    pub fn close_invitation(&self, request: String, reject: bool) -> Result<(), AndroidError> {
        self.command::<serde_json::Value>(Command::CloseInvitation {
            request: super::digest(&request)?,
            reject,
        })
        .map(|_| ())
    }
    /// Read shared policy with typed bounded values.
    pub fn policy(&self) -> Result<PolicyView, AndroidError> {
        let policy: taypeer_core::DatabasePolicy = self.command(Command::DatabasePolicy)?;
        Ok(PolicyView {
            attachment_bytes: policy.attachment_bytes(),
            total_attachment_bytes: policy.total_attachment_bytes(),
            kdf_target_ms: policy.kdf_target_ms(),
        })
    }
    /// Apply an explicitly confirmed policy, retaining the exact operation on retry.
    pub fn set_policy(
        &self,
        policy: PolicyView,
        password: Option<String>,
        operation: String,
    ) -> Result<(), AndroidError> {
        let policy = taypeer_core::DatabasePolicy::new(
            policy.attachment_bytes,
            policy.total_attachment_bytes,
            policy.kdf_target_ms,
        )
        .map_err(|_| AndroidError::InvalidOptions)?;
        self.command::<serde_json::Value>(Command::SetDatabasePolicy {
            policy,
            operation: super::digest(&operation)?,
            password: password.map(|s| Zeroizing::new(s.into_bytes())),
        })
        .map(|_| ())
    }
    /// Rotate access after the client's explicit security confirmation. Revoking a
    /// device and changing the password share one durable authority transition.
    pub fn rotate_password(
        &self,
        password: String,
        revoke: Option<String>,
        operation: String,
    ) -> Result<(), AndroidError> {
        let revoke = revoke
            .map(|id| id.parse().map_err(|_| AndroidError::InvalidOptions))
            .transpose()?;
        self.command::<serde_json::Value>(Command::RotatePassword {
            password: Zeroizing::new(password.into_bytes()),
            revoke,
            operation: operation
                .parse::<Digest>()
                .map_err(|_| AndroidError::InvalidOptions)?,
        })
        .map(|_| ())
    }
}
