//! PUBLIC-only driver for scripts/check-power-cut.sh; never use real credentials.
//! This archive durability baseline omits document semantics and protected anchors.
use std::{fs, io, path::Path};
use taypeer_core::{DatabaseId, SchemaDescriptor};
use taypeer_storage::{ArchiveSeed, ArchiveStore, EncryptedObject, create_epoch};
use taypeer_trust::{AuthorKey, ControlChain, Digest, Identity, ObjectKind, TransportKey};

const PASSWORD: &[u8] = b"PUBLIC power-cut archive password";

fn create(path: &Path) -> Result<ArchiveStore, Box<dyn std::error::Error>> {
    let author = AuthorKey::from_seed(&[41; 32]);
    let transport = TransportKey::from_seed(&[42; 32]);
    let chain = ControlChain::genesis(
        DatabaseId::new("PUBLIC power-cut archive"),
        Identity::new(author.public(), transport.public())?,
        &author,
        Digest::of(b"PUBLIC policy"),
        SchemaDescriptor::current(),
    )?;
    let (header, key) = create_epoch(PASSWORD, 500)?;
    let object = |kind, bytes: &[u8]| {
        EncryptedObject::seal(
            &chain,
            &author,
            kind,
            &header,
            &key,
            bytes,
            bytes.len() as u64,
        )
    };
    let checkpoint = object(ObjectKind::Checkpoint, b"PUBLIC checkpoint")?;
    let baseline = object(ObjectKind::Baseline, b"PUBLIC baseline")?;
    Ok(ArchiveSeed {
        controls: chain.records().to_vec(),
        checkpoint: checkpoint.descriptor().digest,
        baseline: baseline.descriptor().digest,
        objects: vec![checkpoint, baseline],
    }
    .create(path, &transport, None)?)
}

fn replace(path: &Path) -> Result<ArchiveStore, Box<dyn std::error::Error>> {
    let mut store = ArchiveStore::open(path, None, None)?;
    let snapshot = store.snapshot().clone();
    let checkpoint = snapshot.object(snapshot.metadata().manifest.body.checkpoint)?;
    let key = checkpoint.unlock_key(PASSWORD)?;
    let bytes = vec![b'P'; 2 * 1024 * 1024 + 7];
    let blob = EncryptedObject::seal(
        snapshot.chain(),
        &AuthorKey::from_seed(&[41; 32]),
        ObjectKind::Blob,
        &checkpoint.password_header()?,
        &key,
        bytes.as_slice(),
        bytes.len() as u64,
    )?;
    let mut candidate = snapshot.candidate();
    candidate.insert(blob)?;
    let mut body = snapshot.metadata().manifest.body.clone();
    body.generation = body
        .generation
        .checked_add(1)
        .ok_or("generation overflow")?;
    let metadata = candidate.metadata(
        snapshot.chain(),
        &TransportKey::from_seed(&[42; 32]),
        body,
        snapshot.metadata().journal.clone(),
    )?;
    store.commit(snapshot.fingerprint(), &candidate, metadata)?;
    Ok(store)
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arguments: Vec<_> = std::env::args_os().collect();
    if arguments.len() != 4 {
        return Err("usage: power_cut create|replace|verify DIRECTORY OUTSIDE-DISK-ORACLE".into());
    }
    let directory = Path::new(&arguments[2]).canonicalize()?;
    let oracle = Path::new(&arguments[3]);
    let oracle_parent = oracle
        .parent()
        .ok_or("missing oracle parent")?
        .canonicalize()?;
    if oracle_parent.starts_with(&directory) {
        return Err("oracle must live outside the replayed filesystem".into());
    }
    let path = directory.join("PUBLIC.taypeer");
    let phase = arguments[1].to_str().ok_or("invalid phase")?;
    let store = match phase {
        "create" => create(&path)?,
        "replace" => replace(&path)?,
        "verify" => ArchiveStore::open(&path, None, None)?,
        _ => return Err("invalid phase".into()),
    };
    let snapshot = store.snapshot();
    let observed = (
        snapshot.metadata().manifest.body.generation,
        snapshot.fingerprint(),
    );
    if phase == "verify" {
        let expected: (u64, Digest) = serde_json::from_slice(&fs::read(oracle)?)?;
        if observed != expected {
            return Err("acknowledged archive generation/fingerprint was lost".into());
        }
        for id in snapshot.metadata().manifest.body.objects.keys() {
            let object = snapshot.object(*id)?;
            object.decrypt(&object.unlock_key(PASSWORD)?, &mut io::sink())?;
        }
        println!(
            "PASS: recovered generation {} with complete authenticated objects",
            observed.0
        );
    } else {
        // A separate filesystem retains the oracle after ArchiveStore returned ACK.
        fs::write(oracle, serde_json::to_vec(&observed)?)?;
        fs::File::open(oracle)?.sync_all()?;
        fs::File::open(oracle.parent().ok_or("missing oracle parent")?)?.sync_all()?;
    }
    Ok(())
}
