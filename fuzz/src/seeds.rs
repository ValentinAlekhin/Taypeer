use super::*;
use std::io::Read;
use taypeer_storage::LocalStateKey;
use taypeer_trust::{AuthorKey, InvitationSecret, TransportKey};

/// Fixed PUBLIC inputs and signed examples extracted from their verified archive.
/// Construction asserts acceptance before deterministic corruptions are exercised.
pub fn seed_inputs() -> Vec<(Parser, &'static str, Vec<u8>)> {
    let mut seeds = Vec::new();
    for (name, bytes) in [
        (
            "empty",
            include_bytes!("../../tests/fixtures/stable1/empty.taypeer").as_slice(),
        ),
        (
            "populated",
            include_bytes!("../../tests/fixtures/stable1/populated.taypeer").as_slice(),
        ),
        (
            "conflicts",
            include_bytes!("../../tests/fixtures/stable1/conflicts.taypeer").as_slice(),
        ),
    ] {
        assert!(bytes.len() <= MAX_INPUT);
        archive(bytes).expect("fixed PUBLIC archive must validate");
        seeds.push((Parser::Storage, name, bytes.to_vec()));
    }
    let bytes = include_bytes!("../../tests/fixtures/stable1/extensions.automerge");
    let document = Document::load(bytes).expect("fixed PUBLIC document must validate");
    seeds.push((Parser::Document, "extensions", bytes.to_vec()));
    for change in document
        .changes_since(&[])
        .expect("fixed document has original sources")
    {
        OriginalChange::parse(change.bytes()).expect("fixed PUBLIC source must decode");
        seeds.push((Parser::Document, "source", change.bytes().to_vec()));
    }
    let snapshot = archive(include_bytes!(
        "../../tests/fixtures/stable1/conflicts.taypeer"
    ))
    .unwrap();
    let chain = snapshot.chain();
    ControlChain::validate(chain.records().to_vec(), chain.root().unwrap()).unwrap();
    seeds.push((
        Parser::Trust,
        "controls",
        serde_json::to_vec(chain.records()).unwrap(),
    ));
    snapshot.metadata().manifest.verify(chain).unwrap();
    seeds.push((
        Parser::Trust,
        "manifest",
        serde_json::to_vec(&snapshot.metadata().manifest).unwrap(),
    ));
    snapshot.metadata().verify(chain.root().unwrap()).unwrap();
    seeds.push((
        Parser::Storage,
        "metadata",
        serde_json::to_vec(snapshot.metadata()).unwrap(),
    ));
    let object = snapshot
        .object(snapshot.metadata().manifest.body.checkpoint)
        .unwrap();
    object.envelope().verify(chain).unwrap();
    seeds.push((
        Parser::Trust,
        "envelope",
        serde_json::to_vec(object.envelope()).unwrap(),
    ));
    let mut bytes = Vec::new();
    object.reader().unwrap().read_to_end(&mut bytes).unwrap();
    EncryptedObject::open_source(input(&bytes), chain, TemporaryStorage::default()).unwrap();
    seeds.push((Parser::Storage, "object", bytes));
    let author = AuthorKey::from_seed(&[19; 32]);
    let proof = SourceProof::sign(chain, &author, SOURCE).unwrap();
    proof.verify(chain, SOURCE).unwrap();
    seeds.push((
        Parser::Trust,
        "source-proof",
        serde_json::to_vec(&proof).unwrap(),
    ));
    let invitation = Invitation::create(
        chain,
        &author,
        &InvitationSecret::from_bytes([74; 32]),
        1000,
    )
    .unwrap();
    invitation.verify(chain, 1000).unwrap();
    seeds.push((
        Parser::Trust,
        "invitation",
        serde_json::to_vec(&invitation).unwrap(),
    ));
    let enrolled = LocalCredential::new(
        credential_scope(),
        &author,
        &TransportKey::from_seed(&[59; 32]),
        &LocalStateKey::from_secret(&[86; 32]),
    );
    let reader = LocalCredential::read_only(
        credential_scope(),
        &TransportKey::from_seed(&[87; 32]),
        &LocalStateKey::from_secret(&[86; 32]),
    );
    for (name, record, has_author) in [
        ("enrolled-credential", enrolled, true),
        ("reader-credential", reader, false),
    ] {
        let cipher = record.seal(&credential_key()).unwrap();
        let opened =
            LocalCredential::open(&cipher, &credential_key(), &credential_scope()).unwrap();
        assert_eq!(opened.author().is_some(), has_author);
        assert_eq!(opened.transport().public(), record.transport().public());
        assert_eq!(opened.local_state_key().secret_bytes(), &[86; 32]);
        seeds.push((Parser::Storage, name, cipher));
    }
    seeds
}
