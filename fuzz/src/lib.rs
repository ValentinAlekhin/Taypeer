//! Bounded inputs to actual production parsers, shared by seed replay and libFuzzer.
use std::{
    io,
    sync::{Arc, OnceLock},
};
use taypeer_document::{Document, OriginalChange};
use taypeer_storage::{
    ArchiveMetadata, ArchiveSnapshot, CiphertextFile, CiphertextIo, EncryptedObject,
    LocalCredential, LocalCredentialScope, ReadKey, TemporaryStorage,
};
use taypeer_trust::{
    ControlChain, Invitation, ObjectEnvelope, SignedControl, SignedManifest, SourceProof,
};

mod seeds;
pub use seeds::seed_inputs;

/// Harness input budget; this does not establish a production decompression/resource bound.
pub const MAX_INPUT: usize = 1024 * 1024;
const SOURCE: &[u8] = b"PUBLIC fuzz source";

struct Input(Arc<[u8]>);
impl CiphertextIo for Input {
    fn length(&self) -> io::Result<u64> {
        Ok(self.0.len() as u64)
    }
    fn read_at(&self, output: &mut [u8], offset: u64) -> io::Result<usize> {
        let offset = usize::try_from(offset).map_err(|_| io::ErrorKind::InvalidInput)?;
        let available = self.0.get(offset..).unwrap_or_default();
        let count = available.len().min(output.len());
        output[..count].copy_from_slice(&available[..count]);
        Ok(count)
    }
    fn write_at(&self, _: &[u8], _: u64) -> io::Result<usize> {
        Err(io::ErrorKind::PermissionDenied.into())
    }
}
fn input(bytes: &[u8]) -> CiphertextFile {
    CiphertextFile::new(Arc::new(Input(Arc::from(bytes))))
}
fn archive(bytes: &[u8]) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
    ArchiveSnapshot::from_source(input(bytes), None, TemporaryStorage::default())
}
fn chain() -> &'static ControlChain {
    static CHAIN: OnceLock<ControlChain> = OnceLock::new();
    CHAIN.get_or_init(|| {
        archive(include_bytes!(
            "../../tests/fixtures/stable1/conflicts.taypeer"
        ))
        .expect("the frozen PUBLIC seed must validate")
        .chain()
        .clone()
    })
}
fn credential_scope() -> LocalCredentialScope {
    LocalCredentialScope {
        database: chain().head().database.clone(),
        root: chain().root().expect("the fixed PUBLIC chain has a root"),
        epoch: chain().head().epoch,
    }
}
fn credential_key() -> ReadKey {
    ReadKey::from_secret(&[85; 32])
}

/// Concrete parser families; each target uses production decoding and validation.
#[derive(Clone, Copy)]
pub enum Parser {
    /// Automerge document structure and original change decoding.
    Document,
    /// Ciphertext archive, object, signed metadata and keyed local credential.
    Storage,
    /// Signed control, manifest, source proof, envelope and invitation.
    Trust,
}
impl Parser {
    /// Stable cargo-fuzz target and seed-directory name.
    pub fn name(self) -> &'static str {
        match self {
            Self::Document => "document",
            Self::Storage => "storage",
            Self::Trust => "trust",
        }
    }
    /// Exercise bounded arbitrary bytes; malformed input is an ordinary parser result.
    pub fn run(self, bytes: &[u8]) {
        if bytes.len() > MAX_INPUT {
            return;
        }
        match self {
            Self::Document => {
                let _ = OriginalChange::parse(bytes);
                if let Ok(document) = Document::load(bytes) {
                    let reopened = Document::load(&document.export())
                        .expect("a validated document must survive its own export");
                    assert_eq!(document.database_id(), reopened.database_id());
                }
            }
            Self::Storage => {
                let _ = archive(bytes);
                let _ = LocalCredential::open(bytes, &credential_key(), &credential_scope());
                let _ = EncryptedObject::open_source(
                    input(bytes),
                    chain(),
                    TemporaryStorage::default(),
                );
                if let Ok(metadata) = serde_json::from_slice::<ArchiveMetadata>(bytes) {
                    let _ = metadata.verify(chain().root().expect("the seed has a root"));
                }
            }
            Self::Trust => {
                if let Ok(records) = serde_json::from_slice::<Vec<SignedControl>>(bytes) {
                    let _ = ControlChain::validate(
                        records,
                        chain().root().expect("the seed has a root"),
                    );
                }
                if let Ok(manifest) = serde_json::from_slice::<SignedManifest>(bytes) {
                    let _ = manifest.verify(chain());
                }
                if let Ok(envelope) = serde_json::from_slice::<ObjectEnvelope>(bytes) {
                    let _ = envelope.verify(chain());
                }
                if let Ok(proof) = serde_json::from_slice::<SourceProof>(bytes) {
                    let _ = proof.verify(chain(), SOURCE);
                }
                if let Ok(invitation) = serde_json::from_slice::<Invitation>(bytes) {
                    let _ = invitation.verify(chain(), invitation.issued_at);
                }
            }
        }
    }
}
