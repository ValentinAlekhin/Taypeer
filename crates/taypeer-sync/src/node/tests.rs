use super::*;
use crate::{Coordinator, PreparedCommit};
use std::{
    collections::{BTreeMap, BTreeSet},
    sync::atomic::{AtomicBool, Ordering},
};
use taypeer_storage::{
    ArchiveCandidate, ArchiveJournal, ArchiveStore, EncryptedObject, create_epoch,
};
use taypeer_trust::{AuthorKey, ControlChain, ControlTransition, Identity, Manifest, ObjectKind};

struct PublicFixture {
    directory: tempfile::TempDir,
    database: DatabaseId,
    coordinators: Vec<Arc<Coordinator>>,
    keys: Vec<Arc<TransportKey>>,
    blob: Digest,
}
fn fixture() -> PublicFixture {
    fixture_with_schema(taypeer_core::SchemaDescriptor::current())
}
fn fixture_with_schema(schema: taypeer_core::SchemaDescriptor) -> PublicFixture {
    let directory = tempfile::tempdir().unwrap();
    let mut authors = Vec::new();
    let mut keys = Vec::new();
    let mut identities = Vec::new();
    for seed in [51, 53, 55] {
        let author = AuthorKey::from_seed(&[seed; 32]);
        let key = Arc::new(TransportKey::from_seed(&[seed + 1; 32]));
        identities.push(Identity::new(author.public(), key.public()).unwrap());
        authors.push(author);
        keys.push(key);
    }
    let database = DatabaseId::new("PUBLIC network database");
    let mut chain = ControlChain::genesis(
        database.clone(),
        identities[0],
        &authors[0],
        Digest::of(b"PUBLIC policy"),
        schema,
    )
    .unwrap();
    for (i, identity) in identities.iter().enumerate().skip(1) {
        chain = chain
            .transition(
                &authors[0],
                Digest::of(&[i as u8]),
                ControlTransition::Admit(*identity),
            )
            .unwrap();
    }
    let (header, key) = create_epoch(b"PUBLIC network password", 500).unwrap();
    let seal = |kind, bytes: &[u8]| {
        EncryptedObject::seal(
            &chain,
            &authors[0],
            kind,
            &header,
            &key,
            bytes,
            bytes.len() as u64,
        )
        .unwrap()
    };
    let checkpoint = seal(ObjectKind::Checkpoint, b"PUBLIC checkpoint");
    let baseline = seal(ObjectKind::Baseline, b"PUBLIC baseline");
    let blob = seal(ObjectKind::Blob, &vec![b'P'; 2 * 1024 * 1024 + 43]);
    let blob_id = blob.descriptor().digest;
    let mut coordinators = Vec::new();
    for index in 0..3 {
        let mut candidate = ArchiveCandidate::new();
        let checkpoint = candidate.insert(checkpoint.clone()).unwrap();
        let baseline = candidate.insert(baseline.clone()).unwrap();
        let body = Manifest {
            version: 1,
            database: database.clone(),
            trust_set: chain.head().trust_set,
            control: chain.head_hash().unwrap(),
            generation: 0,
            signer: identities[index].device,
            objects: BTreeMap::new(),
            checkpoint,
            baseline,
            auxiliary: Digest::of(&[]),
        };
        let metadata = candidate
            .metadata(&chain, &keys[index], body, ArchiveJournal::default())
            .unwrap();
        let store = ArchiveStore::create(
            &directory.path().join(format!("PUBLIC-{index}.taypeer")),
            &candidate,
            metadata,
            None,
        )
        .unwrap();
        let coordinator = Arc::new(Coordinator::new(Arc::clone(&keys[index])));
        coordinator.register(store).unwrap();
        coordinators.push(coordinator);
    }
    let before = coordinators[0].snapshot(&database).unwrap();
    coordinators[0]
        .commit(
            &database,
            PreparedCommit {
                expected: before.fingerprint(),
                control: chain.head_hash().unwrap(),
                controls: chain.records().to_vec(),
                objects: vec![blob],
                remove: BTreeSet::new(),
                checkpoint: before.metadata().manifest.body.checkpoint,
                baseline: before.metadata().manifest.body.baseline,
                journal: ArchiveJournal::default(),
            },
        )
        .unwrap();
    // Author/read keys die here; listening nodes and coordinators receive transport keys only.
    PublicFixture {
        directory,
        database,
        coordinators,
        keys,
        blob: blob_id,
    }
}
async fn test_node(key: &TransportKey, backend: Arc<dyn Backend>, relay: Option<RelayUrl>) -> Node {
    if let Some(url) = relay {
        assert!(url.as_str().starts_with("https://127.0.0.1:"));
        let seed = key.secret_seed();
        let endpoint = Endpoint::builder(presets::Minimal)
            .secret_key(SecretKey::from_bytes(&seed))
            .relay_mode(RelayMode::Custom(url.into()))
            .clear_ip_transports()
            // This capability exists only inside the test module for its loopback relay.
            .ca_tls_config(iroh_relay::tls::CaTlsConfig::insecure_skip_verify())
            .alpns(vec![ALPN.to_vec()])
            .bind()
            .await
            .unwrap();
        let node = Node::from_endpoint(endpoint, backend);
        node.online().await.unwrap();
        node
    } else {
        Node::bind(key, &RelaySetting::Disabled, backend)
            .await
            .unwrap()
    }
}

async fn forward(relay: Option<RelayUrl>, schema: taypeer_core::SchemaDescriptor) {
    let f = fixture_with_schema(schema.clone());
    let mut nodes = Vec::new();
    for index in 0..3 {
        nodes.push(test_node(&f.keys[index], f.coordinators[index].clone(), relay.clone()).await);
    }
    let sent = nodes[0]
        .exchange(nodes[1].address(), f.database.clone())
        .await
        .unwrap();
    assert_eq!(sent.sent, 1);
    assert_eq!(sent.received, 0);
    let again = nodes[0]
        .exchange(nodes[1].address(), f.database.clone())
        .await
        .unwrap();
    assert_eq!(again, ExchangeReport::default());
    // B is never unlocked but can forward the durably received ciphertext to C.
    let forwarded = nodes[1]
        .exchange(nodes[2].address(), f.database.clone())
        .await
        .unwrap();
    assert_eq!(forwarded.sent, 1);
    let snapshot = f.coordinators[2].snapshot(&f.database).unwrap();
    assert!(
        snapshot
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&f.blob)
    );
    let key = TransportKey::from_seed(&[99; 32]);
    let stranger = test_node(&key, f.coordinators[0].clone(), relay).await;
    assert!(
        stranger
            .request(
                nodes[0].address(),
                Command::Inventory {
                    database: f.database.clone(),
                    address: stranger.address()
                }
            )
            .await
            .is_err()
    );
    stranger.close().await;
    for node in &mut nodes {
        node.close().await;
    }
    for coordinator in &f.coordinators {
        coordinator.unregister(&f.database).unwrap();
    }
    // A fresh reader can authenticate the copied ciphertext with no credential sidecar.
    let copy =
        taypeer_storage::ArchiveSnapshot::open(&f.directory.path().join("PUBLIC-2.taypeer"), None)
            .unwrap();
    assert!(copy.metadata().manifest.body.objects.contains_key(&f.blob));
    let compatibility = copy.compatibility(&taypeer_core::ClientCapabilities::default());
    assert_eq!(compatibility.schema, schema);
    assert!(compatibility.receive.is_supported());
    assert_eq!(
        compatibility.read.is_supported(),
        schema.schema_version() == 5
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn real_direct_transport_forwards_ciphertext_through_a_locked_coordinator() {
    timeout(
        Duration::from_secs(60),
        forward(None, taypeer_core::SchemaDescriptor::current()),
    )
    .await
    .unwrap();
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn forced_loopback_relay_forwards_without_an_ip_transport() {
    let (_, url, _server) = iroh::test_utils::run_relay_server().await.unwrap();
    timeout(
        Duration::from_secs(60),
        forward(Some(url), taypeer_core::SchemaDescriptor::current()),
    )
    .await
    .unwrap();
}

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn unknown_document_schema_can_be_received_and_forwarded_directly_and_over_relay() {
    let schema = taypeer_core::SchemaDescriptor::new(99, BTreeSet::new(), BTreeSet::new()).unwrap();
    timeout(Duration::from_secs(60), forward(None, schema.clone()))
        .await
        .unwrap();
    let (_, url, _server) = iroh::test_utils::run_relay_server().await.unwrap();
    timeout(Duration::from_secs(60), forward(Some(url), schema))
        .await
        .unwrap();
}

struct LostReceipt {
    coordinator: Arc<Coordinator>,
    fail: AtomicBool,
}
impl Backend for LostReceipt {
    fn command(&self, peer: PublicKey, command: Command) -> Result<Reply, Error> {
        self.coordinator.command(peer, command)
    }
    fn authorize_object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: &CipherObject,
    ) -> Result<(), Error> {
        self.coordinator
            .authorize_object(peer, database, descriptor)
    }
    fn object(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        id: Digest,
    ) -> Result<(CipherObject, ObjectReader), Error> {
        self.coordinator.object(peer, database, id)
    }
    fn receive(
        &self,
        peer: PublicKey,
        database: &DatabaseId,
        descriptor: CipherObject,
        file: NamedTempFile,
    ) -> Result<Digest, Error> {
        let id = self.coordinator.receive(peer, database, descriptor, file)?;
        if self.fail.swap(false, Ordering::SeqCst) {
            return Err(Error::Storage(taypeer_storage::Error::CommitUncertain));
        }
        Ok(id)
    }
}
#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
async fn a_lost_durable_receipt_retries_without_duplicating_ciphertext() {
    let f = fixture();
    let a = test_node(&f.keys[0], f.coordinators[0].clone(), None).await;
    let failure = Arc::new(LostReceipt {
        coordinator: f.coordinators[1].clone(),
        fail: AtomicBool::new(true),
    });
    let b = test_node(&f.keys[1], failure, None).await;
    let result = a.exchange(b.address(), f.database.clone()).await;
    assert!(result.is_err());
    let before = f.coordinators[1].snapshot(&f.database).unwrap();
    assert!(
        before
            .metadata()
            .manifest
            .body
            .objects
            .contains_key(&f.blob)
    );
    let report = a.exchange(b.address(), f.database.clone()).await.unwrap();
    assert_eq!(report, ExchangeReport::default());
    assert_eq!(
        before.fingerprint(),
        f.coordinators[1]
            .snapshot(&f.database)
            .unwrap()
            .fingerprint()
    );
    a.close().await;
    b.close().await;
}
