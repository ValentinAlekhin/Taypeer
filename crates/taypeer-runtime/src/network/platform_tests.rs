//! Public synthetic credentials, recipient proofs and actual direct Iroh connections.
use super::*;
use crate::{
    profile::{CredentialStore, ProfileError},
    session::SessionController,
};
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use taypeer_services::{DatabaseService, EntryPatch, FieldUpdate, SessionToken};
use taypeer_trust::{AuthorKey, Identity, InvitationStatus, Signature};

const PASSWORD: &[u8] = b"PUBLIC platform join password";
type CredentialValues = BTreeMap<(String, String), Zeroizing<Vec<u8>>>;

#[derive(Default)]
struct Credentials {
    values: Mutex<CredentialValues>,
    author_reads: AtomicUsize,
    fail_join_save: AtomicBool,
}
impl CredentialStore for Credentials {
    fn get(
        &self,
        service: &str,
        account: &str,
    ) -> Result<Option<Zeroizing<Vec<u8>>>, ProfileError> {
        if account == "author" {
            self.author_reads.fetch_add(1, Ordering::Relaxed);
        }
        Ok(self
            .values
            .lock()
            .unwrap()
            .get(&(service.into(), account.into()))
            .cloned())
    }
    fn set(&self, service: &str, account: &str, bytes: &[u8]) -> Result<(), ProfileError> {
        if account == "state:joins" && self.fail_join_save.load(Ordering::Relaxed) {
            return Err(ProfileError::Io);
        }
        self.values.lock().unwrap().insert(
            (service.into(), account.into()),
            Zeroizing::new(bytes.to_vec()),
        );
        Ok(())
    }
}
struct Peer {
    host: RuntimeHost,
    directory: PathBuf,
    credentials: Arc<Credentials>,
}
impl Peer {
    fn new(directory: PathBuf) -> Self {
        let credentials = Arc::new(Credentials::default());
        let host = Self::host(&directory, &credentials);
        Self {
            host,
            directory,
            credentials,
        }
    }
    fn host(directory: &Path, credentials: &Arc<Credentials>) -> RuntimeHost {
        RuntimeHost::with_platform_credentials(
            directory,
            SessionController::new(Default::default()),
            credentials.clone(),
        )
        .unwrap()
    }
    fn restart(self) -> Self {
        let Self {
            host,
            directory,
            credentials,
        } = self;
        drop(host);
        let host = Self::host(&directory, &credentials);
        Self {
            host,
            directory,
            credentials,
        }
    }
    fn proof(&self, invitation: &Invitation) -> JoinProof {
        // Fixture signer acts as the isolated enrollment process; the host API receives
        // only its public output. Account-read assertions start after this explicit step.
        let author = self.host.profile().author().unwrap();
        let identity =
            Identity::new(author.public(), self.host.profile().transport_public()).unwrap();
        let proof = JoinProof::sign(invitation, identity, &author).unwrap();
        self.credentials.author_reads.store(0, Ordering::Relaxed);
        proof
    }
    fn assert_no_author_read(&self) {
        assert_eq!(self.credentials.author_reads.load(Ordering::Relaxed), 0);
    }
}
struct Fixture {
    directory: tempfile::TempDir,
    manager: Peer,
    service: DatabaseService,
    session: SessionToken,
    source: PathBuf,
}
impl Fixture {
    fn new() -> Self {
        let directory = tempfile::tempdir().unwrap();
        let manager = Peer::new(directory.path().join("PUBLIC-manager-profile"));
        let source = directory.path().join("PUBLIC-manager.taypeer");
        let mut service = DatabaseService::new();
        let session = manager
            .host
            .open_local(
                &mut service,
                &source,
                PASSWORD,
                Some("PUBLIC platform database".into()),
            )
            .unwrap();
        manager.host.start_network(RelaySetting::Disabled).unwrap();
        Self {
            directory,
            manager,
            service,
            session,
            source,
        }
    }
    fn peer(&self, label: &str) -> Peer {
        Peer::new(self.directory.path().join(label))
    }
    fn code(&mut self, session: &SessionToken) -> InvitationCode {
        let (invitation, secret) = self.service.create_invitation(session, now()).unwrap();
        InvitationCode {
            invitation,
            secret: Zeroizing::new(*secret.expose()),
            address: self.manager.host.network_node().unwrap().address(),
        }
    }
    fn request(&mut self, peer: &Peer) -> (InvitationCode, JoinProof, Digest) {
        let code = self.code(&self.session.clone());
        let proof = peer.proof(&code.invitation);
        let request = code.invitation.id().unwrap();
        assert!(matches!(
            peer.host.join_platform_proof(copy_code(&code), proof.clone(), &NetworkCancellation::default()).unwrap(),
            JoinProgress::Pending(id) if id == request
        ));
        assert!(peer.host.working_copies().unwrap().is_empty());
        peer.assert_no_author_read();
        (code, proof, request)
    }
    fn approve(&mut self, request: Digest) {
        self.service
            .approve_invitation(&self.session, request, now())
            .unwrap();
    }
}
fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
fn copy_code(code: &InvitationCode) -> InvitationCode {
    InvitationCode {
        invitation: code.invitation.clone(),
        secret: Zeroizing::new(*code.secret),
        address: code.address.clone(),
    }
}
fn receive(peer: &Peer, code: &InvitationCode, proof: &JoinProof) -> PathBuf {
    assert!(matches!(
        peer.host.join_platform_proof(copy_code(code), proof.clone(), &NetworkCancellation::default()).unwrap(),
        JoinProgress::Received(database) if database == code.invitation.database
    ));
    let copies = peer.host.working_copies().unwrap();
    assert_eq!(copies.len(), 1);
    assert_eq!(copies[0].database, code.invitation.database);
    assert_eq!(copies[0].root, code.invitation.root);
    assert!(
        copies[0]
            .path
            .starts_with(peer.directory.canonicalize().unwrap())
    );
    assert!(peer.host.pending_joins().unwrap().is_empty());
    peer.assert_no_author_read();
    copies[0].path.clone()
}

#[test]
fn public_proof_and_route_are_verified_before_request_mutation() {
    let mut fixture = Fixture::new();
    let peer = fixture.peer("PUBLIC-rejected-profile");
    let code = fixture.code(&fixture.session.clone());
    let proof = peer.proof(&code.invitation);
    let mut bad_route = copy_code(&code);
    bad_route.address = EndpointAddr::new(
        peer.host
            .profile()
            .transport_public()
            .to_string()
            .parse()
            .unwrap(),
    );
    assert!(matches!(
        peer.host
            .join_platform_proof(bad_route, proof.clone(), &NetworkCancellation::default()),
        Err(RuntimeError::Protocol)
    ));
    let mut bad_proof = proof.clone();
    bad_proof.signature = Signature::from_bytes([0; 64]);
    assert!(matches!(
        peer.host
            .join_platform_proof(copy_code(&code), bad_proof, &NetworkCancellation::default()),
        Err(RuntimeError::Protocol)
    ));
    let other_author = AuthorKey::generate().unwrap();
    let other_identity = Identity::new(
        other_author.public(),
        peer.host.profile().transport_public(),
    )
    .unwrap();
    let other_proof = JoinProof::sign(&code.invitation, other_identity, &other_author).unwrap();
    assert!(matches!(
        peer.host.join_platform_proof(
            copy_code(&code),
            other_proof,
            &NetworkCancellation::default()
        ),
        Err(RuntimeError::Protocol)
    ));
    let mut bad_secret = copy_code(&code);
    bad_secret.secret = Zeroizing::new([0; 32]);
    assert!(matches!(
        peer.host
            .join_platform_proof(bad_secret, proof.clone(), &NetworkCancellation::default()),
        Err(RuntimeError::Protocol)
    ));
    let cancelled = NetworkCancellation::default();
    cancelled.cancel();
    assert!(matches!(
        peer.host
            .join_platform_proof(copy_code(&code), proof, &cancelled),
        Err(RuntimeError::Closed)
    ));
    assert!(peer.host.pending_joins().unwrap().is_empty());
    assert!(peer.host.working_copies().unwrap().is_empty());
    assert!(peer.host.network.lock().unwrap().is_empty());
    peer.assert_no_author_read();
    let snapshot = fixture
        .manager
        .host
        .coordinator()
        .snapshot(&fixture.session.database)
        .unwrap();
    assert!(matches!(
        snapshot.metadata().journal.invitations[&code.invitation.id().unwrap()].status,
        InvitationStatus::Available
    ));
}

#[test]
fn local_durability_failure_never_presents_a_request() {
    let mut fixture = Fixture::new();
    let peer = fixture.peer("PUBLIC-durability-profile");
    let code = fixture.code(&fixture.session.clone());
    let proof = peer.proof(&code.invitation);
    let request = code.invitation.id().unwrap();
    peer.credentials
        .fail_join_save
        .store(true, Ordering::Relaxed);
    assert!(matches!(
        peer.host.join_platform_proof(
            copy_code(&code),
            proof.clone(),
            &NetworkCancellation::default()
        ),
        Err(RuntimeError::Profile(ProfileError::Io))
    ));
    assert!(peer.host.pending_joins().unwrap().is_empty());
    assert!(peer.host.network.lock().unwrap().is_empty());
    let snapshot = fixture
        .manager
        .host
        .coordinator()
        .snapshot(&fixture.session.database)
        .unwrap();
    assert!(matches!(
        snapshot.metadata().journal.invitations[&request].status,
        InvitationStatus::Available
    ));
    peer.credentials
        .fail_join_save
        .store(false, Ordering::Relaxed);
    assert!(
        matches!(peer.host.join_platform_proof(copy_code(&code), proof, &NetworkCancellation::default()).unwrap(), JoinProgress::Pending(id) if id == request)
    );
    assert_eq!(peer.host.pending_joins().unwrap().len(), 1);
    peer.assert_no_author_read();
}

#[test]
fn restart_before_initial_presentation_retries_the_durable_intent() {
    let mut fixture = Fixture::new();
    let peer = fixture.peer("PUBLIC-interrupted-presentation-profile");
    let code = fixture.code(&fixture.session.clone());
    let proof = peer.proof(&code.invitation);
    let request = code.invitation.id().unwrap();
    let path = peer
        .host
        .creation_path(&OperationId::new(format!("join:{request}")))
        .unwrap();
    let pending = PendingJoin {
        invitation: code.invitation.clone(),
        proof: proof.clone(),
        address: code.address.clone(),
        path: path.clone(),
    };
    peer.host
        .profile()
        .save_state("joins", &BTreeMap::from([(request, pending)]))
        .unwrap();
    let peer = peer.restart();
    assert!(
        matches!(peer.host.join_platform_proof(copy_code(&code), proof, &NetworkCancellation::default()).unwrap(), JoinProgress::Pending(id) if id == request)
    );
    assert_eq!(peer.host.pending_joins().unwrap()[&request].path, path);
    assert!(!path.exists());
    peer.assert_no_author_read();
    let snapshot = fixture
        .manager
        .host
        .coordinator()
        .snapshot(&fixture.session.database)
        .unwrap();
    assert!(matches!(
        snapshot.metadata().journal.invitations[&request].status,
        InvitationStatus::Requested(_)
    ));
}

#[test]
fn three_peers_restart_resume_internal_copy_and_exchange_original_edits() {
    let mut fixture = Fixture::new();
    let recipient = fixture.peer("PUBLIC-recipient-profile");
    let (code, proof, request) = fixture.request(&recipient);
    fixture.approve(request);
    let source_before = std::fs::read(&fixture.source).unwrap();
    let recipient = recipient.restart();
    let path = receive(&recipient, &code, &proof);
    assert_eq!(std::fs::read(&fixture.source).unwrap(), source_before);
    let fingerprint = taypeer_storage::ArchiveSnapshot::open(&path, None)
        .unwrap()
        .fingerprint();
    assert_eq!(receive(&recipient, &code, &proof), path);
    assert_eq!(
        taypeer_storage::ArchiveSnapshot::open(&path, None)
            .unwrap()
            .fingerprint(),
        fingerprint
    );

    let forwarder = fixture.peer("PUBLIC-forwarder-profile");
    let (forward_code, forward_proof, forward_request) = fixture.request(&forwarder);
    fixture.approve(forward_request);
    let forward_path = receive(&forwarder, &forward_code, &forward_proof);
    // The first recipient receives the later membership while still locked.
    recipient
        .host
        .exchange(
            fixture.manager.host.network_node().unwrap().address(),
            fixture.session.database.clone(),
        )
        .unwrap();
    let mut writer = DatabaseService::new();
    let writer_session = forwarder
        .host
        .open_local(&mut writer, &forward_path, PASSWORD, None)
        .unwrap();
    let entry = writer
        .create_entry_in(
            &writer_session,
            None,
            EntryPatch {
                title: FieldUpdate::Set("PUBLIC original forwarded entry".into()),
                password: FieldUpdate::Set("PUBLIC protected forwarded marker".into()),
                ..Default::default()
            },
            &OperationId::new("PUBLIC forwarded create"),
        )
        .unwrap()
        .value;
    let source = forwarder
        .host
        .coordinator()
        .snapshot(&fixture.session.database)
        .unwrap();
    let original_changes: Vec<_> = source
        .metadata()
        .manifest
        .body
        .objects
        .values()
        .filter(|object| object.kind == taypeer_trust::ObjectKind::Change)
        .map(|object| object.digest)
        .collect();
    assert!(!original_changes.is_empty());
    recipient
        .host
        .exchange(
            forwarder.host.network_node().unwrap().address(),
            fixture.session.database.clone(),
        )
        .unwrap();
    let received = recipient
        .host
        .coordinator()
        .snapshot(&fixture.session.database)
        .unwrap();
    assert!(original_changes.iter().all(|digest| {
        received
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(digest)
    }));
    recipient.assert_no_author_read();
    let mut reader = DatabaseService::new();
    let reader_session = recipient
        .host
        .open_local(&mut reader, &path, PASSWORD, None)
        .unwrap();
    assert!(reader.view_entry(&reader_session, &entry).is_err());
    reader.apply_received(&reader_session).unwrap();
    assert_eq!(
        reader
            .view_entry(&reader_session, &entry)
            .unwrap()
            .value
            .title,
        "PUBLIC original forwarded entry"
    );
    assert!(
        reader
            .view_entry(&reader_session, &entry)
            .unwrap()
            .value
            .has_password
    );
    assert_eq!(
        reader.history(&reader_session, &entry).unwrap().value.len(),
        1
    );
    recipient
        .host
        .exchange(
            forwarder.host.network_node().unwrap().address(),
            fixture.session.database.clone(),
        )
        .unwrap();
    reader.apply_received(&reader_session).unwrap();
    assert_eq!(
        reader.history(&reader_session, &entry).unwrap().value.len(),
        1
    );
    let bytes = std::fs::read(&path).unwrap();
    assert!(
        !bytes
            .windows(b"PUBLIC protected forwarded marker".len())
            .any(|window| window == b"PUBLIC protected forwarded marker")
    );
}

#[test]
fn completing_one_database_keeps_other_pending_invitations_after_restart() {
    let mut fixture = Fixture::new();
    let peer = fixture.peer("PUBLIC-two-database-profile");
    let first_code = fixture.code(&fixture.session.clone());
    let first_proof = peer.proof(&first_code.invitation);
    let first_request = first_code.invitation.id().unwrap();
    let second_session = fixture
        .manager
        .host
        .open_local(
            &mut fixture.service,
            &fixture.directory.path().join("PUBLIC-second.taypeer"),
            PASSWORD,
            Some("PUBLIC second database".into()),
        )
        .unwrap();
    let second_code = fixture.code(&second_session);
    let second_proof = peer.proof(&second_code.invitation);
    let second_request = second_code.invitation.id().unwrap();
    let start = std::sync::Barrier::new(2);
    let present = |code: &InvitationCode, proof: &JoinProof| {
        start.wait();
        peer.host.join_platform_proof(
            copy_code(code),
            proof.clone(),
            &NetworkCancellation::default(),
        )
    };
    std::thread::scope(|threads| {
        let first = threads.spawn(|| present(&first_code, &first_proof));
        let second = threads.spawn(|| present(&second_code, &second_proof));
        assert!(
            matches!(first.join().unwrap().unwrap(), JoinProgress::Pending(id) if id == first_request)
        );
        assert!(
            matches!(second.join().unwrap().unwrap(), JoinProgress::Pending(id) if id == second_request)
        );
    });
    assert_eq!(peer.host.pending_joins().unwrap().len(), 2);
    fixture.approve(first_request);
    let peer = peer.restart();
    assert!(
        matches!(peer.host.join_platform_proof(copy_code(&first_code), first_proof, &NetworkCancellation::default()).unwrap(), JoinProgress::Received(database) if database == first_code.invitation.database)
    );
    let pending = peer.host.pending_joins().unwrap();
    assert_eq!(pending.len(), 1);
    assert!(pending.contains_key(&second_request));
    assert_eq!(peer.host.working_copies().unwrap().len(), 1);
    let peer = peer.restart();
    assert!(
        matches!(peer.host.join_platform_proof(copy_code(&second_code), second_proof, &NetworkCancellation::default()).unwrap(), JoinProgress::Pending(id) if id == second_request)
    );
    peer.assert_no_author_read();
}

#[cfg(target_os = "linux")]
#[test]
fn lazy_linux_profile_rejects_public_platform_proof_without_enrollment() {
    let mut fixture = Fixture::new();
    let peer = fixture.peer("PUBLIC-signing-profile");
    let code = fixture.code(&fixture.session.clone());
    let proof = peer.proof(&code.invitation);
    let lazy = RuntimeHost::new(&fixture.directory.path().join("PUBLIC-lazy-profile")).unwrap();
    assert!(matches!(
        lazy.join_platform_proof(code, proof, &NetworkCancellation::default()),
        Err(RuntimeError::Profile(ProfileError::Credentials))
    ));
    assert!(lazy.pending_join_summaries().unwrap().is_empty());
    assert!(lazy.working_copies().unwrap().is_empty());
    assert!(lazy.network.lock().unwrap().is_empty());
}
