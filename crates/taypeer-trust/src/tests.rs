use crate::*;
use std::collections::BTreeMap;
use taypeer_core::DatabaseId;

fn profile(seed: u8) -> (AuthorKey, TransportKey, Identity) {
    let author = AuthorKey::from_seed(&[seed; 32]);
    let transport = TransportKey::from_seed(&[seed + 1; 32]);
    let identity = Identity::new(author.public(), transport.public()).unwrap();
    (author, transport, identity)
}
fn operation(value: u8) -> Digest {
    Digest::of(&[value])
}
fn genesis(author: &AuthorKey, identity: Identity) -> ControlChain {
    ControlChain::genesis(
        DatabaseId::new("PUBLIC database"),
        identity,
        author,
        operation(0),
        taypeer_core::SchemaDescriptor::current(),
    )
    .unwrap()
}

#[test]
fn admission_revocation_and_readmission_do_not_erase_an_authority_gap() {
    let (a, _, aid) = profile(1);
    let (b, _, bid) = profile(3);
    let chain = genesis(&a, aid);
    assert_eq!(
        chain.transition(&b, operation(1), ControlTransition::Admit(bid)),
        Err(Error::Unauthorized)
    );
    let joined = chain
        .transition(&a, operation(1), ControlTransition::Admit(bid))
        .unwrap();
    let source = SourceProof::sign(&joined, &b, b"PUBLIC original change").unwrap();
    source.verify(&joined, b"PUBLIC original change").unwrap();
    assert!(joined.continuous(bid.device, source.control).unwrap());
    let rotated = joined
        .transition(
            &a,
            operation(2),
            ControlTransition::Rotate {
                revoke: None,
                policy: operation(2),
            },
        )
        .unwrap();
    assert!(rotated.continuous(bid.device, source.control).unwrap());
    let revoked = rotated
        .transition(
            &a,
            operation(3),
            ControlTransition::Rotate {
                revoke: Some(bid.device),
                policy: operation(3),
            },
        )
        .unwrap();
    assert_eq!(
        revoked.admit_transport(bid.transport),
        Err(Error::Unauthorized)
    );
    source.verify(&revoked, b"PUBLIC original change").unwrap();
    assert!(!revoked.continuous(bid.device, source.control).unwrap());
    let rejoined = revoked
        .transition(&a, operation(4), ControlTransition::Admit(bid))
        .unwrap();
    assert!(!rejoined.continuous(bid.device, source.control).unwrap());
    assert_eq!(
        rejoined
            .transition(&a, operation(4), ControlTransition::Admit(bid))
            .unwrap(),
        rejoined
    );
    assert_eq!(
        rejoined.transition(&a, operation(4), ControlTransition::Policy(operation(9))),
        Err(Error::OperationMismatch)
    );
    assert!(source.verify(&rejoined, b"PUBLIC changed bytes").is_err());
    let encoded = serde_json::to_vec(rejoined.records()).unwrap();
    let reopened = ControlChain::validate(
        serde_json::from_slice(&encoded).unwrap(),
        chain.root().unwrap(),
    )
    .unwrap();
    assert_eq!(reopened, rejoined);
}

#[test]
fn handoff_requires_bound_recipient_consent_and_forks_remain_distinct() {
    let (a, _, aid) = profile(5);
    let (b, _, bid) = profile(7);
    let base = genesis(&a, aid)
        .transition(&a, operation(1), ControlTransition::Admit(bid))
        .unwrap();
    let consent = HandoffConsent::sign(&base, &b, operation(2)).unwrap();
    let handed = base
        .transition(
            &a,
            operation(2),
            ControlTransition::Transfer(consent.clone()),
        )
        .unwrap();
    assert_eq!(handed.head().manager, bid.device);
    assert_eq!(
        handed.transition(&a, operation(3), ControlTransition::Policy(operation(3))),
        Err(Error::Unauthorized)
    );
    let after = handed
        .transition(&b, operation(3), ControlTransition::Policy(operation(3)))
        .unwrap();
    assert_eq!(base.reconcile(&after).unwrap(), after);
    let competing = base
        .transition(&a, operation(4), ControlTransition::Policy(operation(4)))
        .unwrap();
    assert_eq!(after.reconcile(&competing), Err(Error::Fork));
    assert!(
        competing
            .transition(&a, operation(2), ControlTransition::Transfer(consent))
            .is_err()
    );
    let mut forged = after.records().to_vec();
    forged.last_mut().unwrap().body.manager = aid.device;
    assert!(ControlChain::validate(forged, base.root().unwrap()).is_err());
}

#[test]
fn invitation_binds_actual_peer_and_survives_equal_request_retries_only() {
    let (a, _, aid) = profile(9);
    let (b, _, bid) = profile(11);
    let (_, _, cid) = profile(13);
    let chain = genesis(&a, aid);
    let secret = InvitationSecret::from_bytes([42; 32]);
    let invitation = Invitation::create(&chain, &a, &secret, 1000).unwrap();
    let proof = JoinProof::sign(&invitation, bid, &b).unwrap();
    let mut state = InvitationStatus::Available;
    assert!(
        state
            .request(
                &invitation,
                &secret,
                proof.clone(),
                cid.transport,
                &chain,
                1001
            )
            .is_err()
    );
    assert_eq!(state, InvitationStatus::Available);
    state
        .request(
            &invitation,
            &secret,
            proof.clone(),
            bid.transport,
            &chain,
            1001,
        )
        .unwrap();
    state
        .request(
            &invitation,
            &secret,
            proof.clone(),
            bid.transport,
            &chain,
            1299,
        )
        .unwrap();
    assert_eq!(
        state.request(
            &invitation,
            &secret,
            proof.clone(),
            bid.transport,
            &chain,
            1300
        ),
        Err(Error::InvitationUnavailable)
    );
    state = InvitationStatus::Rejected;
    assert!(
        state
            .request(&invitation, &secret, proof, bid.transport, &chain, 1002)
            .is_err()
    );
    assert!(invitation.verify(&chain, 999).is_err());
    let rotated = chain
        .transition(
            &a,
            operation(1),
            ControlTransition::Rotate {
                revoke: None,
                policy: operation(1),
            },
        )
        .unwrap();
    assert_eq!(invitation.verify(&rotated, 1002), Err(Error::Stale));
}

#[test]
fn transport_manifest_signature_never_substitutes_for_an_author_proof() {
    let (a, transport, identity) = profile(15);
    let chain = genesis(&a, identity);
    let checkpoint = operation(1);
    let baseline = operation(2);
    let manifest = Manifest {
        version: 1,
        database: chain.head().database.clone(),
        trust_set: chain.head().trust_set,
        control: chain.head_hash().unwrap(),
        generation: 1,
        signer: identity.device,
        objects: BTreeMap::from([
            (
                checkpoint,
                CipherObject {
                    digest: checkpoint,
                    length: 40,
                    kind: ObjectKind::Checkpoint,
                },
            ),
            (
                baseline,
                CipherObject {
                    digest: baseline,
                    length: 40,
                    kind: ObjectKind::Baseline,
                },
            ),
        ]),
        checkpoint,
        baseline,
        auxiliary: operation(3),
    };
    let signed = SignedManifest::sign(manifest, &chain, &transport).unwrap();
    signed.verify(&chain).unwrap();
    let mut changed = signed.clone();
    changed.body.objects.remove(&baseline);
    assert!(changed.verify(&chain).is_err());
    let mut future = signed.clone();
    future.body.version = 99;
    assert_eq!(future.verify(&chain), Err(Error::UnsupportedVersion));
    let mut future =
        ObjectEnvelope::sign(&chain, &a, ObjectKind::Checkpoint, 40, baseline).unwrap();
    future.version = 99;
    assert_eq!(future.verify(&chain), Err(Error::UnsupportedVersion));
    let proof = SourceProof::sign(&chain, &a, b"PUBLIC bytes").unwrap();
    let mut forged = proof.clone();
    // Even a valid signature from the admitted network key is not an author signature.
    forged.signature = signed.signature;
    assert!(forged.verify(&chain, b"PUBLIC bytes").is_err());
    assert!(!format!("{proof:?}").contains(&proof.change.to_string()));
    let (member_key, _, member) = profile(21);
    let joined = chain
        .transition(&a, operation(5), ControlTransition::Admit(member))
        .unwrap();
    assert!(
        ObjectEnvelope::sign(
            &joined,
            &member_key,
            ObjectKind::Checkpoint,
            40,
            operation(1)
        )
        .is_ok()
    );
    assert_eq!(
        ObjectEnvelope::sign(&joined, &member_key, ObjectKind::Baseline, 40, operation(1)),
        Err(Error::Unauthorized)
    );
}

#[test]
fn identities_validate_encoding_and_separate_key_roles() {
    let (a, _, identity) = profile(17);
    assert_eq!(identity.device, a.device_id());
    assert!(Identity::new(a.public(), a.public()).is_err());
    assert!(serde_json::from_str::<Digest>("\"bad\"").is_err());
    let mut value = serde_json::to_value(identity).unwrap();
    value["device"] = serde_json::json!(Digest::of(b"PUBLIC wrong identity").to_string());
    let decoded: Identity = serde_json::from_value(value).unwrap();
    assert!(decoded.validate().is_err());
}

#[test]
fn schema_requirements_are_signed_and_cannot_change_during_administration() {
    let (author, _, identity) = profile(25);
    let chain = genesis(&author, identity);
    let original = chain.head().schema.clone();
    let mut writes = original.required_write_features().clone();
    writes.insert(taypeer_core::FeatureId::new("future.retention").unwrap());
    let changed =
        taypeer_core::SchemaDescriptor::new(5, original.required_read_features().clone(), writes)
            .unwrap();
    let mut root = chain.records()[0].clone();
    root.body.schema = changed.clone();
    assert_eq!(
        ControlChain::validate(vec![root.clone()], root.hash().unwrap()),
        Err(Error::Signature)
    );
    let next = chain
        .transition(
            &author,
            operation(60),
            ControlTransition::Policy(operation(61)),
        )
        .unwrap();
    let mut records = next.records().to_vec();
    records[1].body.schema = changed;
    records[1].signature = author.sign(b"taypeer/control/2", &records[1].body).unwrap();
    assert_eq!(
        ControlChain::validate(records, chain.root().unwrap()),
        Err(Error::Invalid)
    );
    let mut encoded = serde_json::to_value(chain.records()).unwrap();
    encoded[0]["body"]["schema"]["required_write_features"] = serde_json::json!([]);
    assert!(serde_json::from_value::<Vec<SignedControl>>(encoded).is_err());
    let mut root = chain.records()[0].clone();
    root.body.version = 99;
    assert_eq!(
        ControlChain::validate(vec![root.clone()], root.hash().unwrap()),
        Err(Error::UnsupportedVersion)
    );
    let mut source = SourceProof::sign(&chain, &author, b"PUBLIC source").unwrap();
    source.version = 99;
    assert_eq!(
        source.verify(&chain, b"PUBLIC source"),
        Err(Error::UnsupportedVersion)
    );
    source.version = 1;
    source.schema = 99;
    assert_eq!(source.verify(&chain, b"PUBLIC source"), Err(Error::Invalid));
}
