//! Epoch credentials are decrypted only by the isolated document worker.
use super::*;
use crate::profile::TransportCapability;
use taypeer_storage::{LocalCredential, LocalCredentialScope, LocalStateKey, ReadKey};
use taypeer_trust::{AuthorKey, TransportKey};
use zeroize::Zeroizing;

pub(super) fn scope(snapshot: &ArchiveSnapshot) -> Result<LocalCredentialScope, RuntimeError> {
    Ok(LocalCredentialScope {
        database: snapshot.chain().head().database.clone(),
        root: snapshot
            .chain()
            .root()
            .map_err(|_| RuntimeError::Protocol)?,
        epoch: snapshot.chain().head().epoch,
    })
}

pub(super) fn capability(record: &LocalCredential) -> TransportCapability {
    let transport = record.transport();
    TransportCapability {
        database: record.scope().database.clone(),
        root: record.scope().root,
        transport_seed: transport.secret_seed(),
        local_state_seed: Some(Zeroizing::new(*record.local_state_key().secret_bytes())),
        enrolled: record.author().is_some(),
    }
}

pub(super) fn create(
    document: &dyn PlatformDocument,
    seed: &ArchiveSeed,
    key: &ReadKey,
    author: &AuthorKey,
    transport: &TransportKey,
) -> Result<LocalCredential, RuntimeError> {
    let control = seed.controls.last().ok_or(RuntimeError::Protocol)?;
    let root = seed
        .controls
        .first()
        .ok_or(RuntimeError::Protocol)?
        .hash()
        .map_err(|_| RuntimeError::Protocol)?;
    let record = LocalCredential::new(
        LocalCredentialScope {
            database: control.body.database.clone(),
            root,
            epoch: control.body.epoch,
        },
        author,
        transport,
        &LocalStateKey::generate().map_err(crate::cipher_ipc::storage)?,
    );
    document.publish_credential(
        record.scope(),
        &record.seal(key).map_err(crate::cipher_ipc::storage)?,
        false,
    )?;
    document.activate_credentials(capability(&record), None)?;
    Ok(record)
}

pub(super) fn enrollment(
    document: &dyn PlatformDocument,
    invitation: &taypeer_trust::Invitation,
    password: &[u8],
) -> Result<LocalCredential, RuntimeError> {
    let scope = LocalCredentialScope {
        database: invitation.database.clone(),
        root: invitation.root,
        epoch: 0,
    };
    let record = match document.credential(&scope, true)? {
        Some(cipher) => LocalCredential::open_with_password(&cipher, password, &scope)
            .map_err(crate::cipher_ipc::storage)?,
        None => {
            let record = LocalCredential::new(
                scope,
                &AuthorKey::generate().map_err(|_| RuntimeError::Protocol)?,
                &TransportKey::generate().map_err(|_| RuntimeError::Protocol)?,
                &LocalStateKey::generate().map_err(crate::cipher_ipc::storage)?,
            );
            document.publish_credential(
                record.scope(),
                &record
                    .seal_with_password(password, 1000)
                    .map_err(crate::cipher_ipc::storage)?,
                true,
            )?;
            record
        }
    };
    document.activate_credentials(capability(&record), None)?;
    Ok(record)
}

pub(super) fn bind_enrollment(
    document: &dyn PlatformDocument,
    record: &mut LocalCredential,
    password: &[u8],
) -> Result<(), RuntimeError> {
    let snapshot = document.snapshot().map_err(crate::cipher_ipc::storage)?;
    let expected = scope(&snapshot)?;
    let key = DatabaseService::authenticate_archive_credentials(&snapshot, password)?;
    let identity = taypeer_trust::Identity::new(
        record.author().ok_or(RuntimeError::Protocol)?.public(),
        record.transport().public(),
    )
    .map_err(|_| RuntimeError::Protocol)?;
    if expected.database != record.scope().database
        || expected.root != record.scope().root
        || snapshot
            .chain()
            .head()
            .members
            .get(&identity.device)
            .is_none_or(|member| member.identity != identity)
    {
        return Err(taypeer_services::ServiceError::Unauthorized.into());
    }
    record.set_epoch(expected.epoch);
    document.publish_credential(
        &expected,
        &record.seal(&key).map_err(crate::cipher_ipc::storage)?,
        true,
    )?;
    document.activate_credentials(capability(record), Some(snapshot.fingerprint()))?;
    document.finalize_credential(&expected, true)
}

pub(super) fn open(
    document: &dyn PlatformDocument,
    snapshot: &ArchiveSnapshot,
    key: &ReadKey,
) -> Result<Option<LocalCredential>, RuntimeError> {
    DatabaseService::authenticate_archive_read_key(snapshot, key)?;
    let expected = scope(snapshot)?;
    let active = document.credential(&expected, false)?;
    let prepared = document.credential(&expected, true)?;
    if active.is_none() && prepared.is_none() {
        let transport = TransportKey::generate().map_err(|_| RuntimeError::Protocol)?;
        let record = LocalCredential::read_only(
            expected,
            &transport,
            &LocalStateKey::generate().map_err(crate::cipher_ipc::storage)?,
        );
        document.publish_credential(
            record.scope(),
            &record.seal(key).map_err(crate::cipher_ipc::storage)?,
            false,
        )?;
        document.activate_credentials(capability(&record), Some(snapshot.fingerprint()))?;
        document.finalize_credential(record.scope(), false)?;
        return Ok(Some(record));
    }
    let mut selected = None;
    let mut promote = false;
    let mut last = taypeer_storage::Error::Authentication;
    for (is_prepared, cipher) in prepared
        .iter()
        .map(|cipher| (true, cipher))
        .chain(active.iter().map(|cipher| (false, cipher)))
    {
        match LocalCredential::open(cipher, key, &expected) {
            Ok(record) => {
                selected = Some(record);
                promote = is_prepared;
                break;
            }
            Err(error) => last = error,
        }
    }
    if selected.is_none() {
        let history = DatabaseService::authenticate_archive_keyring_with_key(snapshot, key)?;
        'history: for (epoch, old) in history {
            if epoch == expected.epoch {
                continue;
            }
            let historic = LocalCredentialScope {
                epoch,
                ..expected.clone()
            };
            for cipher in prepared.iter().chain(active.iter()) {
                if let Ok(mut record) = LocalCredential::open(cipher, &old, &historic) {
                    record.set_epoch(expected.epoch);
                    document.publish_credential(
                        &expected,
                        &record.seal(key).map_err(crate::cipher_ipc::storage)?,
                        true,
                    )?;
                    selected = Some(record);
                    promote = true;
                    break 'history;
                }
            }
        }
    }
    let record = selected.ok_or(taypeer_services::ServiceError::Storage(last))?;
    document.activate_credentials(capability(&record), Some(snapshot.fingerprint()))?;
    // The host must authenticate its encrypted state before acknowledging the epoch.
    document.finalize_credential(&expected, promote)?;
    let current = document.snapshot().map_err(crate::cipher_ipc::storage)?;
    if current.fingerprint() != snapshot.fingerprint() {
        return Err(crate::cipher_ipc::storage(taypeer_storage::Error::Changed));
    }
    Ok(Some(record))
}

pub(super) fn rotate(
    service: &mut DatabaseService,
    session: &taypeer_services::SessionToken,
    document: &dyn PlatformDocument,
    record: &mut LocalCredential,
    operation: Digest,
    password: &[u8],
    revoke: Option<taypeer_trust::DeviceId>,
) -> Result<serde_json::Value, RuntimeError> {
    service.rotate_password_with_credentials(
        session,
        operation,
        password,
        revoke,
        |stage, _old, new, epoch| credential_stage(document, record, stage, new, epoch),
    )?;
    Ok(serde_json::Value::Null)
}

pub(super) fn policy(
    service: &mut DatabaseService,
    session: &taypeer_services::SessionToken,
    document: &dyn PlatformDocument,
    record: &mut LocalCredential,
    operation: Digest,
    policy: taypeer_core::DatabasePolicy,
    password: Option<&[u8]>,
) -> Result<serde_json::Value, RuntimeError> {
    service.set_database_policy_with_credentials(
        session,
        operation,
        policy,
        password,
        |stage, _old, new, epoch| credential_stage(document, record, stage, new, epoch),
    )?;
    Ok(serde_json::Value::Null)
}

fn credential_stage(
    document: &dyn PlatformDocument,
    record: &mut LocalCredential,
    stage: taypeer_services::EpochCredentialStage,
    key: &ReadKey,
    epoch: u64,
) -> Result<(), taypeer_services::ServiceError> {
    use taypeer_services::EpochCredentialStage;
    record.set_epoch(epoch);
    let result = (|| match stage {
        EpochCredentialStage::Prepare => document.publish_credential(
            record.scope(),
            &record.seal(key).map_err(crate::cipher_ipc::storage)?,
            true,
        ),
        EpochCredentialStage::Finalize => {
            let mut promote = false;
            let mut valid = false;
            for prepared in [true, false] {
                if let Some(cipher) = document.credential(record.scope(), prepared)?
                    && let Ok(saved) = LocalCredential::open(&cipher, key, record.scope())
                    && saved.author().map(|author| author.public())
                        == record.author().map(|author| author.public())
                    && saved.transport().public() == record.transport().public()
                    && saved.local_state_key().secret_bytes()
                        == record.local_state_key().secret_bytes()
                {
                    valid = true;
                    promote = prepared;
                    break;
                }
            }
            if !valid {
                return Err(crate::cipher_ipc::storage(
                    taypeer_storage::Error::Authentication,
                ));
            }
            document.finalize_credential(record.scope(), promote)
        }
    })();
    result.map_err(|error| match error {
        RuntimeError::Service(error) => error,
        RuntimeError::Profile(crate::profile::ProfileError::CommitUncertain) => {
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::CommitUncertain)
        }
        RuntimeError::Profile(crate::profile::ProfileError::Io) => {
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::Io)
        }
        _ => taypeer_services::ServiceError::Credentials,
    })
}
