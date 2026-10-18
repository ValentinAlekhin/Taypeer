//! Opening returns a public challenge before any key/author/session exists.
use super::*;
use crate::cipher_ipc::Channel;
use std::sync::{Arc, Mutex};
use taypeer_services::SessionToken;

pub(in crate::platform_worker) fn open_keyed(
    channel: &Mutex<Channel>,
    service: &mut DatabaseService,
    document: Arc<dyn PlatformDocument>,
    inputs: Option<&dyn BiometricInputs>,
    lease: BiometricInputLease,
    credential: &mut Option<taypeer_storage::LocalCredential>,
) -> Result<Option<SessionToken>, RuntimeError> {
    lease.require(BiometricInputPurpose::KeyedBoot)?;
    let inputs = inputs.ok_or(taypeer_services::ServiceError::Unauthorized)?;
    require_scope(&current_scope(document.as_ref())?, &lease)?;
    let _retire = RetireInput {
        inputs,
        lease: &lease,
    };
    inputs.arm(&lease)?;
    inputs.check(&lease)?;
    channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .response(serde_json::to_value(&lease).map_err(|_| RuntimeError::Protocol))?;
    // This idle read holds no OS/input/control lock. The host runs its prompt
    // separately; Lock remains a valid request while no plaintext session exists.
    let request: Incoming = channel
        .lock()
        .map_err(|_| RuntimeError::Transport)?
        .read()?;
    match request {
        Incoming::Command(crate::Command::Lock) => {
            channel
                .lock()
                .map_err(|_| RuntimeError::Transport)?
                .response(Ok(serde_json::Value::Null))?;
            return Ok(None);
        }
        Incoming::Biometry(Envelope {
            biometry: Request::CompleteBoot(expected),
        }) if expected == lease => {}
        _ => return Err(RuntimeError::Protocol),
    }
    inputs.check(&lease)?;
    require_scope(&current_scope(document.as_ref())?, &lease)?;
    let key = super::super::read_private_key_input(inputs.consume(&lease)?)?;
    inputs.check(&lease)?;
    require_scope(&current_scope(document.as_ref())?, &lease)?;
    if document.portable_credentials() {
        let snapshot = document.snapshot().map_err(crate::cipher_ipc::storage)?;
        *credential = super::super::credentials::open(document.as_ref(), &snapshot, &key)?;
        inputs.check(&lease)?;
        require_scope(&current_scope(document.as_ref())?, &lease)?;
    }
    let port = Box::new(super::super::Persistence(Arc::clone(&document)));
    let mut author_failure = None;
    let session = service.open_managed_with_key_and_author(port, key, |snapshot| {
        let result = (|| {
            inputs.check(&lease)?;
            require_scope(
                &DatabaseService::archive_key_scope(snapshot, document.working_copy())?,
                &lease,
            )?;
            require_scope(&current_scope(document.as_ref())?, &lease)?;
            let author = if document.portable_credentials() {
                credential
                    .as_ref()
                    .and_then(taypeer_storage::LocalCredential::author)
            } else {
                document.author(Some(snapshot.fingerprint()))?
            };
            inputs.check(&lease)?;
            require_scope(&current_scope(document.as_ref())?, &lease)?;
            Ok(author)
        })();
        result.map_err(|error| {
            author_failure = Some(error);
            match error {
                RuntimeError::Service(error) => error,
                _ => taypeer_services::ServiceError::Credentials,
            }
        })
    });
    if let Some(error) = author_failure {
        return Err(error);
    }
    let session = session?;
    inputs.check(&lease)?;
    require_scope(&current_scope(document.as_ref())?, &lease)?;
    if service.can_write(&session)? {
        service.apply_received(&session)?;
    }
    inputs.check(&lease)?;
    Ok(Some(session))
}
