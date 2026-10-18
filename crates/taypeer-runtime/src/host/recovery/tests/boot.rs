//! The production private worker loop consumes actual encrypted candidates; no password bypass.
use super::*;
use crate::platform_worker::{PlatformDocument, PlatformRecoveryBootstrap};
use std::{
    io::{self, Cursor, Write},
    sync::atomic::{AtomicUsize, Ordering},
};
use taypeer_storage::{CipherPersistence, EncryptedObject, PreparedCommit, TemporaryStorage};

struct Adapter {
    writer: Arc<PlatformCipherWriter>,
    author_calls: AtomicUsize,
}
fn storage(error: RuntimeError) -> taypeer_storage::Error {
    match error {
        RuntimeError::Service(taypeer_services::ServiceError::Storage(error)) => error,
        _ => taypeer_storage::Error::Io,
    }
}
impl CipherPersistence for Adapter {
    fn snapshot(&self) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.writer.snapshot().map_err(storage)
    }
    fn commit(&self, _request: PreparedCommit) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        Err(taypeer_storage::Error::Busy)
    }
    fn working_copy(&self) -> Digest {
        self.writer.working_copy().unwrap()
    }
    fn save_draft(&self, _object: &EncryptedObject) -> Result<(), taypeer_storage::Error> {
        Err(taypeer_storage::Error::Busy)
    }
    fn load_draft(
        &self,
        _chain: &ControlChain,
    ) -> Result<Option<EncryptedObject>, taypeer_storage::Error> {
        Ok(None)
    }
    fn discard_draft(&self) -> Result<(), taypeer_storage::Error> {
        Ok(())
    }
    fn path(&self) -> &Path {
        Path::new("")
    }
}
impl PlatformDocument for Adapter {
    fn portable_credentials(&self) -> bool {
        false
    }
    fn temporary(&self) -> TemporaryStorage {
        self.writer.context.ciphertext_staging()
    }
    fn create(&self, seed: ArchiveSeed) -> Result<(), RuntimeError> {
        self.writer.create(seed)
    }
    fn author(&self, authenticated: Option<Digest>) -> Result<Option<AuthorKey>, RuntimeError> {
        self.author_calls.fetch_add(1, Ordering::SeqCst);
        self.writer.author(authenticated).map(Some)
    }
    fn transport_public(&self) -> Result<taypeer_trust::PublicKey, RuntimeError> {
        Ok(self.writer.transport_public())
    }
    fn recovery_bootstrap(
        &self,
        operation: Digest,
    ) -> Result<Option<PlatformRecoveryBootstrap>, RuntimeError> {
        let Some((source, original)) = self.writer.recovery_snapshot(operation)? else {
            return Ok(None);
        };
        let candidate = self.writer.recovery_boot_candidate(&source)?;
        let (identity, candidate) = match candidate {
            Some((intent, candidate)) => (Some(intent.identity), Some(candidate)),
            None => (None, None),
        };
        Ok(Some(PlatformRecoveryBootstrap {
            source,
            original,
            candidate,
            identity,
        }))
    }
    fn freeze_recovery(
        &self,
        operation: Digest,
        authenticated: Digest,
    ) -> Result<(RecoverySource, ArchiveSnapshot), RuntimeError> {
        self.writer.freeze_recovery(operation, authenticated)
    }
    fn author_recovery(&self, source: &RecoverySource) -> Result<AuthorKey, RuntimeError> {
        self.author_calls.fetch_add(1, Ordering::SeqCst);
        self.writer.author_recovery(source)
    }
    fn recovery_candidate(
        &self,
        intent: &RecoveryIntent,
    ) -> Result<Option<ArchiveSnapshot>, RuntimeError> {
        self.writer.recovery_candidate(intent)
    }
    fn publish_recovery(
        &self,
        intent: &RecoveryIntent,
        seed: ArchiveSeed,
    ) -> Result<RecoveryPublication, RuntimeError> {
        self.writer.publish_recovery(intent, seed)
    }
    fn confirm_recovery(&self, publication: &RecoveryPublication) -> Result<(), RuntimeError> {
        self.writer.confirm_recovery(publication)
    }
}
#[derive(Clone)]
struct Capture(Arc<Mutex<Vec<u8>>>);
impl Write for Capture {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
fn run(
    adapter: Arc<Adapter>,
    operation: Digest,
    password: &str,
    commands: Vec<crate::Command>,
) -> Vec<Result<serde_json::Value, RuntimeError>> {
    let mut input = Vec::new();
    crate::protocol::write_frame(&mut input, &serde_json::json!({ "password": password, "form": null, "invitation": null, "recovery": operation })).unwrap();
    for command in commands {
        crate::protocol::write_frame(&mut input, &command).unwrap();
    }
    let output = Capture(Arc::new(Mutex::new(Vec::new())));
    crate::platform_worker::run_platform_worker(Cursor::new(input), output.clone(), adapter)
        .unwrap();
    let bytes = output.0.lock().unwrap().clone();
    let mut cursor = Cursor::new(bytes);
    let mut responses = Vec::new();
    while cursor.position() < cursor.get_ref().len() as u64 {
        let frame: crate::cipher_ipc::WorkerMessage =
            crate::protocol::read_frame(&mut cursor).unwrap();
        match frame {
            crate::cipher_ipc::WorkerMessage::Response(response) => {
                responses.push(response.into_result())
            }
            crate::cipher_ipc::WorkerMessage::Io(_) => {
                panic!("PUBLIC test adapter must not use host streams")
            }
        }
    }
    responses
}
fn adapter(fixture: &Fixture) -> Arc<Adapter> {
    Arc::new(Adapter {
        writer: fixture.host.platform_cipher_writer(&fixture.path).unwrap(),
        author_calls: AtomicUsize::new(0),
    })
}

#[test]
fn candidate_password_alone_resumes_exact_receipt_and_wrong_input_does_not_poison_retry() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(Digest::of(b"PUBLIC candidate-only worker restart"));
    let value = fixture
        .writer
        .publish_recovery(&prepared.intent, prepared.seed)
        .unwrap();
    let before = fixture
        .writer
        .recovery_candidate(&value.intent)
        .unwrap()
        .unwrap()
        .fingerprint();
    let adapter = adapter(&fixture);
    let replies = run(
        adapter.clone(),
        value.intent.source.operation,
        "PUBLIC new recovery password",
        vec![
            crate::Command::RecoverWorkingCopy {
                operation: value.intent.source.operation,
                name: "PUBLIC wrong changed name".into(),
                password: Zeroizing::new(b"PUBLIC new recovery password".to_vec()),
            },
            crate::Command::RecoverWorkingCopy {
                operation: value.intent.source.operation,
                name: prepared.named.name,
                password: Zeroizing::new(b"PUBLIC new recovery password".to_vec()),
            },
            crate::Command::Lock,
        ],
    );
    assert!(
        replies[0].is_ok(),
        "PUBLIC recovery boot failed: {:?}",
        replies[0].as_ref().err()
    );
    assert_eq!(replies.len(), 4);
    assert!(matches!(
        replies[1],
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Trust(taypeer_trust::Error::OperationMismatch)
        ))
    ));
    assert_eq!(
        serde_json::from_value::<RecoveryPublication>(replies[2].as_ref().unwrap().clone())
            .unwrap(),
        value
    );
    assert!(replies[3].is_ok());
    assert_eq!(adapter.author_calls.load(Ordering::SeqCst), 2);
    assert_eq!(
        fixture
            .writer
            .recovery_candidate(&value.intent)
            .unwrap()
            .unwrap()
            .fingerprint(),
        before
    );
}

#[test]
fn obsolete_source_password_cannot_boot_a_published_candidate_or_acquire_author() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(Digest::of(
        b"PUBLIC old password is not candidate authentication",
    ));
    let value = fixture
        .writer
        .publish_recovery(&prepared.intent, prepared.seed)
        .unwrap();
    let adapter = adapter(&fixture);
    let replies = run(
        adapter.clone(),
        value.intent.source.operation,
        "PUBLIC original password",
        Vec::new(),
    );
    assert_eq!(replies.len(), 1);
    assert!(matches!(
        replies[0],
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::Authentication)
        ))
    ));
    assert_eq!(adapter.author_calls.load(Ordering::SeqCst), 0);
}

#[test]
fn a_self_consistent_candidate_cannot_choose_a_foreign_current_profile_author() {
    let fixture = Fixture::new();
    let prepared = fixture.prepare(Digest::of(b"PUBLIC identity binding before worker author"));
    let value = fixture
        .writer
        .publish_recovery(&prepared.intent, prepared.seed)
        .unwrap();
    let foreign = Identity::new(
        AuthorKey::from_seed(&[85; 32]).public(),
        fixture.writer.transport_public(),
    )
    .unwrap();
    let mut values = fixture.credentials.values.lock().unwrap();
    let identity_key = values
        .keys()
        .find(|(_, account)| account == "identity")
        .unwrap()
        .clone();
    values.insert(identity_key, serde_json::to_vec(&foreign).unwrap());
    drop(values);
    let adapter = adapter(&fixture);
    let replies = run(
        adapter.clone(),
        value.intent.source.operation,
        "PUBLIC new recovery password",
        Vec::new(),
    );
    assert_eq!(replies.len(), 1);
    assert!(matches!(
        replies[0],
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Trust(taypeer_trust::Error::OperationMismatch)
        ))
    ));
    assert_eq!(adapter.author_calls.load(Ordering::SeqCst), 0);
}
