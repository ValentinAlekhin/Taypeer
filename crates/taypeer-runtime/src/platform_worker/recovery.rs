//! One worker-owned named attempt. A retained candidate never follows the current document port.
use super::*;
use crate::{RecoveryIntent, RecoveryPublication, RecoverySource};
use taypeer_services::{
    NamedRecoveryIntent, SessionToken, TrustRecoveryPreparation, TrustRecoverySource,
};
use zeroize::Zeroizing;

pub(super) struct WorkingRecovery {
    operation: Digest,
    name: String,
    password: Zeroizing<Vec<u8>>,
    captured: Option<TrustRecoverySource>,
    frozen: Option<(RecoverySource, ArchiveSnapshot)>,
    prepared: Option<TrustRecoveryPreparation>,
    seed: Option<ArchiveSeed>,
}
pub(super) struct Request {
    pub(super) operation: Digest,
    pub(super) name: String,
    pub(super) password: Zeroizing<Vec<u8>>,
}
pub(super) fn resume(
    source: RecoverySource,
    original: ArchiveSnapshot,
    name: String,
    password: &[u8],
) -> WorkingRecovery {
    WorkingRecovery {
        operation: source.operation,
        name,
        password: Zeroizing::new(password.to_vec()),
        captured: None,
        frozen: Some((source, original)),
        prepared: None,
        seed: None,
    }
}
fn mismatch() -> RuntimeError {
    taypeer_services::ServiceError::Trust(taypeer_trust::Error::OperationMismatch).into()
}
fn named(intent: &RecoveryIntent, name: &str) -> NamedRecoveryIntent {
    NamedRecoveryIntent {
        operation: intent.source.operation,
        database: intent.source.database.clone(),
        working_copy: intent.source.working_copy,
        root: intent.source.root,
        source: intent.source.fingerprint,
        identity: intent.identity,
        name: name.to_owned(),
    }
}
fn seed_copy(seed: &ArchiveSeed) -> ArchiveSeed {
    // Only immutable ciphertext leases are cloned; retries keep the same genesis and exact objects.
    ArchiveSeed {
        controls: seed.controls.clone(),
        objects: seed.objects.clone(),
        checkpoint: seed.checkpoint,
        baseline: seed.baseline,
    }
}

pub(super) fn recover(
    service: &DatabaseService,
    session: &SessionToken,
    document: &dyn PlatformDocument,
    pending: &mut Option<WorkingRecovery>,
    request: Request,
) -> Result<serde_json::Value, RuntimeError> {
    let Request {
        operation,
        name,
        password,
    } = request;
    if let Some(prior) = pending.as_ref() {
        if prior.operation != operation
            || prior.name != name
            || prior.password.as_slice() != password.as_slice()
        {
            return Err(mismatch());
        }
    } else {
        TrustRecoverySource::validate_fields(&name, &password)?;
        // Capture before the first host I/O; failed staging cannot replace these contents.
        let captured = service.freeze_trust_recovery_source(session)?;
        captured.validate_input(&name, &password)?;
        *pending = Some(WorkingRecovery {
            operation,
            name,
            password,
            captured: Some(captured),
            frozen: None,
            prepared: None,
            seed: None,
        });
    }
    let state = pending.as_mut().ok_or(RuntimeError::Protocol)?;
    if state.frozen.is_none() {
        let original = state
            .captured
            .as_ref()
            .ok_or(RuntimeError::Protocol)?
            .snapshot();
        state.frozen = Some(document.freeze_recovery(operation, original.fingerprint())?);
    }
    let (source, snapshot) = state.frozen.as_ref().ok_or(RuntimeError::Protocol)?;
    let author = document.author_recovery(source)?;
    let identity = taypeer_trust::Identity::new(author.public(), document.transport_public()?)
        .map_err(|_| RuntimeError::Protocol)?;
    let intent = RecoveryIntent {
        source: source.clone(),
        identity,
    };
    let exact = named(&intent, &state.name);
    let existing = document.recovery_candidate(&intent)?;
    let candidate = if let Some(existing) = existing {
        existing
    } else {
        if state.prepared.is_none() {
            let prepared = state
                .captured
                .as_ref()
                .ok_or(RuntimeError::Protocol)?
                .bind_named(exact.clone(), &state.password)?;
            state.prepared = Some(prepared);
            state.captured = None;
        }
        let prepared = state.prepared.as_ref().ok_or(RuntimeError::Protocol)?;
        if !prepared.matches(&exact, &state.password) {
            return Err(mismatch());
        }
        if state.seed.is_none() {
            state.seed = Some(prepared.prepare(&author)?);
        }
        let seed = state.seed.as_ref().ok_or(RuntimeError::Protocol)?;
        document.publish_recovery(&intent, seed_copy(seed))?;
        document
            .recovery_candidate(&intent)?
            .ok_or(RuntimeError::Protocol)?
    };
    let root = taypeer_services::validate_named_trust_recovery(
        snapshot,
        &candidate,
        &exact,
        &state.password,
    )?;
    let publication = RecoveryPublication {
        intent,
        root,
        fingerprint: candidate.fingerprint(),
    };
    document.confirm_recovery(&publication)?;
    serde_json::to_value(publication).map_err(|_| RuntimeError::Protocol)
}
