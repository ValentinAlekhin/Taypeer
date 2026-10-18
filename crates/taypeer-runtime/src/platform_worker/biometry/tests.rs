//! PUBLIC Rust protocol/owned-pipe proof; these tests do not claim Android OS authentication.
use super::*;
use crate::{
    RuntimeHost,
    cipher_ipc::WorkerMessage,
    platform_worker::{
        PlatformCipherWriter, run_platform_worker, run_platform_worker_with_biometry,
    },
    profile::{CredentialStore, ProfileError},
    protocol::{read_frame, write_frame},
    session::SessionController,
};
use rsa::{Oaep, RsaPrivateKey, pkcs8::EncodePublicKey};
use std::{
    collections::BTreeMap,
    fs::File,
    io::{self, Cursor, Write},
    path::Path,
    sync::{
        Arc, LazyLock, Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use taypeer_services::SessionToken;
use taypeer_storage::{
    ArchiveSeed, ArchiveSnapshot, CipherPersistence, EncryptedObject, PreparedCommit,
    TemporaryStorage,
};
use taypeer_trust::{AuthorKey, ControlChain, PublicKey};
use zeroize::Zeroizing;

const PASSWORD: &[u8] = b"PUBLIC biometric worker master";
static RSA: LazyLock<RsaPrivateKey> =
    LazyLock::new(|| RsaPrivateKey::new(&mut rand_core::OsRng, 2048).unwrap());

#[derive(Default)]
struct Credentials(Mutex<BTreeMap<(String, String), Vec<u8>>>);
impl CredentialStore for Credentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        Ok(self
            .0
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned()
            .map(Zeroizing::new))
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        self.0
            .lock()
            .unwrap()
            .insert((service.into(), account.into()), bytes.to_vec());
        Ok(())
    }
}
struct Document {
    writer: Arc<PlatformCipherWriter>,
    authors: AtomicUsize,
}
fn storage(error: RuntimeError) -> taypeer_storage::Error {
    match error {
        RuntimeError::Service(taypeer_services::ServiceError::Storage(error)) => error,
        _ => taypeer_storage::Error::Io,
    }
}
impl CipherPersistence for Document {
    fn snapshot(&self) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.writer.snapshot().map_err(storage)
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        self.writer.commit(request).map_err(storage)
    }
    fn working_copy(&self) -> Digest {
        self.writer.working_copy().unwrap()
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), taypeer_storage::Error> {
        self.writer.save_draft(object).map_err(storage)
    }
    fn load_draft(
        &self,
        _: &ControlChain,
    ) -> Result<Option<EncryptedObject>, taypeer_storage::Error> {
        self.writer.load_draft().map_err(storage)
    }
    fn discard_draft(&self) -> Result<(), taypeer_storage::Error> {
        self.writer.discard_draft().map_err(storage)
    }
    fn path(&self) -> &Path {
        Path::new("")
    }
}
impl PlatformDocument for Document {
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
        self.authors.fetch_add(1, Ordering::AcqRel);
        self.writer.author(authenticated).map(Some)
    }
    fn transport_public(&self) -> Result<PublicKey, RuntimeError> {
        Ok(self.writer.transport_public())
    }
}
struct Fixture {
    _directory: tempfile::TempDir,
    host: RuntimeHost,
    service: DatabaseService,
    session: SessionToken,
    document: Arc<Document>,
    path: std::path::PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let host = RuntimeHost::with_platform_credentials(
            &directory.path().join("profile"),
            SessionController::new(Default::default()),
            Arc::new(Credentials::default()),
        )
        .unwrap();
        let path = directory.path().join("PUBLIC biometric worker.taypeer");
        let mut service = DatabaseService::new();
        let session = host
            .open_local(
                &mut service,
                &path,
                PASSWORD,
                Some("PUBLIC biometric worker".into()),
            )
            .unwrap();
        service
            .create_group(
                &session,
                "PUBLIC encrypted group".into(),
                None,
                &taypeer_services::new_operation_id().unwrap(),
            )
            .unwrap();
        let writer = host.platform_cipher_writer(&path).unwrap();
        let document = Arc::new(Document {
            writer,
            authors: AtomicUsize::new(0),
        });
        Self {
            _directory: directory,
            host,
            service,
            session,
            document,
            path,
        }
    }
    fn lease(&self, purpose: BiometricInputPurpose) -> BiometricInputLease {
        BiometricInputLease {
            scope: BiometricAttemptScope {
                namespace: Digest::of(b"PUBLIC biometric profile namespace"),
                managed: current_scope(self.document.as_ref()).unwrap(),
                generation: 1,
                attempt: Digest::of(b"PUBLIC biometric worker attempt"),
            },
            input: Digest::of(b"PUBLIC biometric private input"),
            purpose,
        }
    }
    fn key(&self) -> Zeroizing<Vec<u8>> {
        let snapshot = self.document.writer.snapshot().unwrap();
        let key = DatabaseService::authenticate_archive_credentials(&snapshot, PASSWORD).unwrap();
        Zeroizing::new(key.secret_bytes().to_vec())
    }
}
type ReadHook = Box<dyn FnOnce() + Send>;
struct Input {
    expected: BiometricInputLease,
    reader: Mutex<Option<File>>,
    hook: Mutex<Option<ReadHook>>,
    armed: AtomicBool,
    consumed: AtomicBool,
    retired: Arc<AtomicBool>,
    reads: AtomicUsize,
}
impl Input {
    fn new(expected: BiometricInputLease, bytes: &[u8]) -> Arc<Self> {
        let (reader, writer) = rustix::pipe::pipe().unwrap();
        let mut writer = File::from(writer);
        writer.write_all(bytes).unwrap();
        drop(writer);
        Arc::new(Self {
            expected,
            reader: Mutex::new(Some(File::from(reader))),
            hook: Mutex::new(None),
            armed: AtomicBool::new(false),
            consumed: AtomicBool::new(false),
            retired: Arc::new(AtomicBool::new(false)),
            reads: AtomicUsize::new(0),
        })
    }
}
struct HookReader {
    reader: File,
    hook: Option<ReadHook>,
}
impl Read for HookReader {
    fn read(&mut self, bytes: &mut [u8]) -> io::Result<usize> {
        let count = self.reader.read(bytes)?;
        if count == 0
            && let Some(hook) = self.hook.take()
        {
            hook();
        }
        Ok(count)
    }
}
impl BiometricInputs for Input {
    fn arm(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError> {
        if lease != &self.expected
            || self.retired.load(Ordering::Acquire)
            || self.armed.swap(true, Ordering::AcqRel)
        {
            return Err(RuntimeError::Protocol);
        }
        Ok(())
    }
    fn check(&self, lease: &BiometricInputLease) -> Result<(), RuntimeError> {
        if lease != &self.expected
            || !self.armed.load(Ordering::Acquire)
            || self.retired.load(Ordering::Acquire)
        {
            return Err(RuntimeError::Closed);
        }
        Ok(())
    }
    fn consume(&self, lease: &BiometricInputLease) -> Result<Box<dyn Read + Send>, RuntimeError> {
        self.check(lease)?;
        if self.consumed.swap(true, Ordering::AcqRel) {
            return Err(RuntimeError::Protocol);
        }
        self.reads.fetch_add(1, Ordering::AcqRel);
        let reader = self
            .reader
            .lock()
            .unwrap()
            .take()
            .ok_or(RuntimeError::Transport)?;
        Ok(Box::new(HookReader {
            reader,
            hook: self.hook.lock().unwrap().take(),
        }))
    }
    fn retire(&self, _: &BiometricInputLease) {
        self.retired.store(true, Ordering::Release);
        self.reader.lock().unwrap().take();
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
    document: Arc<Document>,
    input: Option<Arc<Input>>,
    boot: serde_json::Value,
    requests: Vec<serde_json::Value>,
) -> Vec<Result<serde_json::Value, RuntimeError>> {
    let mut bytes = Vec::new();
    write_frame(&mut bytes, &boot).unwrap();
    for request in requests {
        write_frame(&mut bytes, &request).unwrap();
    }
    let output = Capture(Arc::new(Mutex::new(Vec::new())));
    match input {
        Some(input) => {
            run_platform_worker_with_biometry(Cursor::new(bytes), output.clone(), document, input)
                .unwrap()
        }
        None => run_platform_worker(Cursor::new(bytes), output.clone(), document).unwrap(),
    }
    let bytes = output.0.lock().unwrap().clone();
    let mut cursor = Cursor::new(bytes);
    let mut replies = Vec::new();
    while cursor.position() < cursor.get_ref().len() as u64 {
        match read_frame::<WorkerMessage>(&mut cursor).unwrap() {
            WorkerMessage::Response(response) => replies.push(response.into_result()),
            WorkerMessage::Io(_) => panic!("PUBLIC biometric fixture uses no generic key IO"),
        }
    }
    replies
}
fn request(request: Request) -> serde_json::Value {
    serde_json::to_value(Envelope { biometry: request }).unwrap()
}
fn password_boot() -> serde_json::Value {
    serde_json::json!({"password": std::str::from_utf8(PASSWORD).unwrap(), "form": null})
}
fn keyed_boot(lease: &BiometricInputLease) -> serde_json::Value {
    serde_json::json!({"password": "", "form": null, "biometry": lease})
}
fn lock() -> serde_json::Value {
    serde_json::to_value(crate::Command::Lock).unwrap()
}

#[test]
fn password_enrollment_wraps_the_actual_current_key_and_proves_one_owned_pipe_without_mutation() {
    let fixture = Fixture::new();
    let lease = fixture.lease(BiometricInputPurpose::EnrollmentProof);
    let key = fixture.key();
    let input = Input::new(lease.clone(), &key);
    let spki = RSA.to_public_key().to_public_key_der().unwrap();
    let before = std::fs::read(&fixture.path).unwrap();
    let replies = run(
        fixture.document.clone(),
        Some(input.clone()),
        password_boot(),
        vec![
            request(Request::PrepareEnrollment {
                lease: lease.clone(),
                spki: spki.as_bytes().to_vec(),
            }),
            request(Request::ConfirmEnrollment(lease.clone())),
            request(Request::ConfirmEnrollment(lease.clone())),
            lock(),
        ],
    );
    assert_eq!(replies.len(), 5);
    assert!(replies.iter().all(Result::is_ok));
    let wrapped: BiometricWrappedKey =
        serde_json::from_value(replies[1].as_ref().unwrap().clone()).unwrap();
    let clear = Zeroizing::new(
        RSA.decrypt(
            Oaep::new_with_mgf_hash::<sha2::Sha256, sha1::Sha1>(),
            &wrapped.ciphertext,
        )
        .unwrap(),
    );
    assert!(clear.as_slice() == key.as_slice());
    assert_eq!(wrapped.ciphertext.len(), 256);
    assert_eq!(wrapped.lease, lease);
    assert_eq!(input.reads.load(Ordering::Acquire), 1);
    assert_eq!(fixture.document.authors.load(Ordering::Acquire), 1);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
}

#[test]
fn keyed_boot_uses_current_pipe_key_and_cannot_claim_password_enrollment_provenance() {
    let fixture = Fixture::new();
    let lease = fixture.lease(BiometricInputPurpose::KeyedBoot);
    let input = Input::new(lease.clone(), &fixture.key());
    let before = std::fs::read(&fixture.path).unwrap();
    let replies = run(
        fixture.document.clone(),
        Some(input.clone()),
        keyed_boot(&lease),
        vec![
            request(Request::CompleteBoot(lease.clone())),
            serde_json::to_value(crate::Command::DatabaseInfo).unwrap(),
            request(Request::EnrollmentScope),
            lock(),
        ],
    );
    assert_eq!(replies.len(), 5);
    assert_eq!(
        serde_json::from_value::<BiometricInputLease>(replies[0].as_ref().unwrap().clone())
            .unwrap(),
        lease
    );
    assert!(replies[1].is_ok() && replies[2].is_ok() && replies[4].is_ok());
    assert!(matches!(
        replies[3],
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Unauthorized
        ))
    ));
    assert_eq!(fixture.document.authors.load(Ordering::Acquire), 1);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
}

#[test]
fn wrong_or_non_exact_pipe_keys_never_reach_author_or_mutate_the_archive() {
    for bytes in [vec![], vec![0x50; 31], vec![0x50; 32], vec![0x50; 33]] {
        let fixture = Fixture::new();
        let lease = fixture.lease(BiometricInputPurpose::KeyedBoot);
        let before = std::fs::read(&fixture.path).unwrap();
        let replies = run(
            fixture.document.clone(),
            Some(Input::new(lease.clone(), &bytes)),
            keyed_boot(&lease),
            vec![request(Request::CompleteBoot(lease))],
        );
        assert_eq!(replies.len(), 2);
        assert!(replies[0].is_ok() && replies[1].is_err());
        assert_eq!(fixture.document.authors.load(Ordering::Acquire), 0);
        assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
    }
}

#[test]
fn independent_cancel_after_key_eof_refuses_session_before_author() {
    let fixture = Fixture::new();
    let lease = fixture.lease(BiometricInputPurpose::KeyedBoot);
    let input = Input::new(lease.clone(), &fixture.key());
    let fence = Arc::clone(&input.retired);
    *input.hook.lock().unwrap() = Some(Box::new(move || fence.store(true, Ordering::Release)));
    let before = std::fs::read(&fixture.path).unwrap();
    let replies = run(
        fixture.document.clone(),
        Some(input),
        keyed_boot(&lease),
        vec![request(Request::CompleteBoot(lease))],
    );
    assert!(matches!(replies[1], Err(RuntimeError::Closed)));
    assert_eq!(fixture.document.authors.load(Ordering::Acquire), 0);
    assert_eq!(std::fs::read(&fixture.path).unwrap(), before);
}

#[test]
fn actual_checkpoint_change_during_private_read_refuses_before_author() {
    let fixture = Fixture::new();
    let lease = fixture.lease(BiometricInputPurpose::KeyedBoot);
    let input = Input::new(lease.clone(), &fixture.key());
    let Fixture {
        document,
        mut service,
        session,
        path,
        host,
        _directory,
    } = fixture;
    *input.hook.lock().unwrap() = Some(Box::new(move || {
        service
            .create_group(
                &session,
                "PUBLIC changed during biometric read".into(),
                None,
                &taypeer_services::new_operation_id().unwrap(),
            )
            .unwrap();
    }));
    let before = std::fs::read(&path).unwrap();
    let replies = run(
        document.clone(),
        Some(input),
        keyed_boot(&lease),
        vec![request(Request::CompleteBoot(lease))],
    );
    assert!(matches!(
        replies[1],
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Storage(taypeer_storage::Error::Changed)
        ))
    ));
    assert_eq!(document.authors.load(Ordering::Acquire), 0);
    assert_ne!(std::fs::read(&path).unwrap(), before);
    drop(host);
    drop(_directory);
}

#[test]
fn challenge_lock_and_unconfigured_routes_cannot_open_or_consume_an_input() {
    let fixture = Fixture::new();
    let lease = fixture.lease(BiometricInputPurpose::KeyedBoot);
    let input = Input::new(lease.clone(), &fixture.key());
    let replies = run(
        fixture.document.clone(),
        Some(input.clone()),
        keyed_boot(&lease),
        vec![lock()],
    );
    assert_eq!(replies.len(), 2);
    assert!(replies.iter().all(Result::is_ok));
    assert_eq!(input.reads.load(Ordering::Acquire), 0);
    assert!(input.retired.load(Ordering::Acquire));
    assert_eq!(fixture.document.authors.load(Ordering::Acquire), 0);
    let replies = run(
        fixture.document.clone(),
        None,
        keyed_boot(&lease),
        Vec::new(),
    );
    assert!(matches!(
        replies[0],
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Unauthorized
        ))
    ));
    let replies = run(
        fixture.document.clone(),
        None,
        password_boot(),
        vec![
            request(Request::EnrollmentScope),
            serde_json::to_value(crate::Command::DatabaseInfo).unwrap(),
            lock(),
        ],
    );
    assert!(matches!(
        replies[1],
        Err(RuntimeError::Service(
            taypeer_services::ServiceError::Unauthorized
        ))
    ));
    assert!(replies[0].is_ok() && replies[2].is_ok() && replies[3].is_ok());
}

#[test]
fn bounded_spki_accepts_only_exact_rsa2048_f4_and_hides_wrapper_debug() {
    let key = taypeer_storage::ReadKey::from_secret(&[0x50; 32]);
    let spki = RSA.to_public_key().to_public_key_der().unwrap();
    assert_eq!(wrapping::wrap(&key, spki.as_bytes()).unwrap().len(), 256);
    assert_eq!(
        validate_biometric_spki(spki.as_bytes()).unwrap(),
        Digest::of(spki.as_bytes())
    );
    for bytes in [
        vec![],
        vec![0x50; 4097],
        b"PUBLIC malformed SPKI".to_vec(),
        [spki.as_bytes(), &[0]].concat(),
    ] {
        assert!(wrapping::wrap(&key, &bytes).is_err());
        assert!(validate_biometric_spki(&bytes).is_err());
    }
    let short = RsaPrivateKey::new(&mut rand_core::OsRng, 1024)
        .unwrap()
        .to_public_key()
        .to_public_key_der()
        .unwrap();
    assert!(wrapping::wrap(&key, short.as_bytes()).is_err());
    assert!(validate_biometric_spki(short.as_bytes()).is_err());
    let wrong_exponent = rsa::RsaPublicKey::new(
        rsa::traits::PublicKeyParts::n(&RSA.to_public_key()).clone(),
        rsa::BigUint::from(3u32),
    )
    .unwrap()
    .to_public_key_der()
    .unwrap();
    assert!(wrapping::wrap(&key, wrong_exponent.as_bytes()).is_err());
    assert!(validate_biometric_spki(wrong_exponent.as_bytes()).is_err());
    let fixture = Fixture::new();
    let wrapped = BiometricWrappedKey {
        lease: fixture.lease(BiometricInputPurpose::EnrollmentProof),
        spki: Digest::of(spki.as_bytes()),
        ciphertext: vec![0x50; 256],
    };
    assert!(!format!("{wrapped:?}").contains("80"));
}
