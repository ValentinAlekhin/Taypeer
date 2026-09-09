//! Disposable experimental protocol. See DESIGN.md; not a production format.
pub mod container;
pub mod kdf;
pub mod session;
pub mod storage;
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    KeyInit, XChaCha20Poly1305, XNonce,
    aead::{Aead, Payload},
};
use ed25519_dalek::{Signature, Signer, SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    fs::{self, File, OpenOptions},
    io::Write,
    path::Path,
};
use zeroize::Zeroizing;

pub type Result<T> = std::result::Result<T, &'static str>;
pub const MAX_BYTES: usize = 1024 * 1024;
pub const MIN_MEMORY: u32 = 65536;
pub type Id = [u8; 32];

pub fn random<const N: usize>() -> [u8; N] {
    let mut bytes = [0; N];
    OsRng.fill_bytes(&mut bytes);
    bytes
}
pub fn digest(bytes: &[u8]) -> Id {
    Sha256::digest(bytes).into()
}
pub fn derive(
    password: &[u8],
    salt: &[u8; 16],
    memory: u32,
    iterations: u32,
) -> Result<Zeroizing<[u8; 32]>> {
    if password.is_empty()
        || !(MIN_MEMORY..=262144).contains(&memory)
        || !(3..=256).contains(&iterations)
    {
        return Err("KDF policy");
    }
    let params = Params::new(memory, iterations, 1, Some(32)).map_err(|_| "KDF params")?;
    let mut key = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, &mut *key)
        .map_err(|_| "KDF failed")?;
    Ok(key)
}

/// Candidate portable payload contains no device signing key or admission secret.
#[derive(Serialize, Deserialize)]
pub struct Snapshot {
    pub group: [u8; 16],
    pub epoch: u64,
    pub read_key: [u8; 32],
    pub document: Vec<u8>,
    pub applied: BTreeSet<Id>,
}
// All synthetic fixture payloads are <= 1 MiB. A product format needs chunking.
// Header: magic[8], version u16, m u32, t u32, salt[16], nonce[24], ciphertext.
pub fn seal(snapshot: &Snapshot, password: &[u8]) -> Result<Vec<u8>> {
    let salt = random();
    let nonce: [u8; 24] = random();
    let mut header = b"P2PSPIKE".to_vec();
    header.extend(1u16.to_le_bytes());
    header.extend(MIN_MEMORY.to_le_bytes());
    header.extend(3u32.to_le_bytes());
    header.extend(salt);
    header.extend(nonce);
    let key = derive(password, &salt, MIN_MEMORY, 3)?;
    let clear = Zeroizing::new(serde_json::to_vec(snapshot).map_err(|_| "encode")?);
    if clear.len() + header.len() + 16 > MAX_BYTES {
        return Err("size");
    }
    let ciphertext = XChaCha20Poly1305::new((&*key).into())
        .encrypt(
            XNonce::from_slice(&nonce),
            Payload {
                msg: &clear,
                aad: &header,
            },
        )
        .map_err(|_| "encrypt")?;
    header.extend(ciphertext);
    Ok(header)
}
pub fn open(bytes: &[u8], password: &[u8]) -> Result<Snapshot> {
    if bytes.len() < 74 || bytes.len() > MAX_BYTES {
        return Err("size");
    }
    if &bytes[..8] != b"P2PSPIKE" || bytes[8..10] != 1u16.to_le_bytes() {
        return Err("version");
    }
    let memory = u32::from_le_bytes(bytes[10..14].try_into().unwrap());
    let iterations = u32::from_le_bytes(bytes[14..18].try_into().unwrap());
    let salt = bytes[18..34].try_into().unwrap();
    let key = derive(password, &salt, memory, iterations)?;
    let clear = Zeroizing::new(
        XChaCha20Poly1305::new((&*key).into())
            .decrypt(
                XNonce::from_slice(&bytes[34..58]),
                Payload {
                    msg: &bytes[58..],
                    aad: &bytes[..58],
                },
            )
            .map_err(|_| "authentication")?,
    );
    serde_json::from_slice(&clear).map_err(|_| "payload")
}

#[derive(Clone, Serialize, Deserialize)]
pub struct Envelope {
    pub version: u16,
    pub group: [u8; 16],
    pub epoch: u64,
    pub author: Id,
    pub nonce: [u8; 24],
    pub ciphertext: Vec<u8>,
    pub signature: Vec<u8>,
}
impl Envelope {
    fn aad(&self) -> Vec<u8> {
        let mut b = b"Pass2P/spike/change/1".to_vec();
        b.extend(self.version.to_le_bytes());
        b.extend(self.group);
        b.extend(self.epoch.to_le_bytes());
        b.extend(self.author);
        b.extend(self.nonce);
        b
    }
    fn signed(&self) -> Vec<u8> {
        let mut b = self.aad();
        b.extend(&self.ciphertext);
        b
    }
    pub fn create(
        group: [u8; 16],
        epoch: u64,
        key: &[u8; 32],
        author: &SigningKey,
        change: &[u8],
    ) -> Self {
        let mut e = Self {
            version: 1,
            group,
            epoch,
            author: author.verifying_key().to_bytes(),
            nonce: random(),
            ciphertext: vec![],
            signature: vec![],
        };
        e.ciphertext = XChaCha20Poly1305::new(key.into())
            .encrypt(
                XNonce::from_slice(&e.nonce),
                Payload {
                    msg: change,
                    aad: &e.aad(),
                },
            )
            .unwrap();
        e.signature = author.sign(&e.signed()).to_bytes().to_vec();
        e
    }
    pub fn encode(&self) -> Vec<u8> {
        serde_json::to_vec(self).unwrap()
    }
    pub fn decrypt(&self, key: &[u8; 32]) -> Result<Zeroizing<Vec<u8>>> {
        XChaCha20Poly1305::new(key.into())
            .decrypt(
                XNonce::from_slice(&self.nonce),
                Payload {
                    msg: &self.ciphertext,
                    aad: &self.aad(),
                },
            )
            .map(Zeroizing::new)
            .map_err(|_| "authentication")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Fault {
    None,
    BeforeWrite,
    PartialWrite,
    BeforeRename,
    AfterRename,
    BeforeDirectorySync,
    CrashBeforeRename,
    CrashAfterRename,
}
pub fn atomic_save(path: &Path, bytes: &[u8], fault: Fault) -> Result<()> {
    use std::os::unix::fs::OpenOptionsExt;
    let parent = path.parent().ok_or("parent")?;
    for candidate in [
        path.to_path_buf(),
        path.with_extension("pending"),
        path.with_extension("previous"),
    ] {
        if fs::symlink_metadata(candidate).is_ok_and(|m| m.file_type().is_symlink()) {
            return Err("symlink");
        }
    }
    if fault == Fault::BeforeWrite {
        return Err("injected write");
    }
    // Single-writer model: production needs writer locking and unique temp names.
    let temp = path.with_extension("pending");
    let previous = path.with_extension("previous");
    if path.exists() {
        fs::copy(path, &previous).map_err(|_| "backup")?;
        File::open(&previous)
            .and_then(|f| storage::sync_file(&f))
            .map_err(|_| "backup sync")?;
        File::open(parent)
            .and_then(|f| f.sync_all())
            .map_err(|_| "backup directory sync")?;
    }
    let mut file = OpenOptions::new()
        .create(true)
        .truncate(true)
        .write(true)
        .mode(0o600)
        .custom_flags(libc::O_NOFOLLOW)
        .open(&temp)
        .map_err(|_| "write")?;
    if fault == Fault::PartialWrite {
        file.write_all(&bytes[..bytes.len() / 2])
            .map_err(|_| "write")?;
        return Err("injected partial write");
    }
    file.write_all(bytes)
        .and_then(|_| storage::sync_file(&file))
        .map_err(|_| "write sync")?;
    if fault == Fault::BeforeRename {
        return Err("injected before rename");
    }
    if fault == Fault::CrashBeforeRename {
        std::process::exit(77);
    }
    fs::rename(&temp, path).map_err(|_| "rename")?;
    if fault == Fault::CrashAfterRename {
        std::process::exit(77);
    }
    if fault == Fault::AfterRename || fault == Fault::BeforeDirectorySync {
        return Err("injected durability uncertainty");
    }
    File::open(parent)
        .and_then(|f| f.sync_all())
        .map_err(|_| "directory sync")?;
    Ok(())
}

/// This type intentionally has no key capable of decrypting a change.
pub struct LockedInbox {
    pub group: [u8; 16],
    pub epoch: u64,
    pub members: BTreeSet<Id>,
    pub directory: std::path::PathBuf,
}
impl LockedInbox {
    fn validate(&self, bytes: &[u8]) -> Result<Envelope> {
        if bytes.len() > MAX_BYTES {
            return Err("size");
        }
        let e: Envelope = serde_json::from_slice(bytes).map_err(|_| "envelope")?;
        if e.version != 1 || e.group != self.group || e.epoch != self.epoch {
            return Err("state");
        }
        if !self.members.contains(&e.author) {
            return Err("author revoked");
        }
        let vk = VerifyingKey::from_bytes(&e.author).map_err(|_| "author")?;
        let signature = Signature::from_slice(&e.signature).map_err(|_| "signature")?;
        vk.verify_strict(&e.signed(), &signature)
            .map_err(|_| "signature")?;
        Ok(e)
    }
    /// peer is a model input for an already authenticated transport connection.
    /// Return means durable received ACK; never means applied.
    pub fn receive(&self, peer: Id, bytes: &[u8], fault: Fault) -> Result<Id> {
        if !self.members.contains(&peer) {
            return Err("no admission");
        }
        let e = self.validate(bytes)?;
        // Normalize encoding so whitespace or JSON field order cannot change object ID.
        let canonical = e.encode();
        let id = digest(&canonical);
        let name: String = id.iter().map(|x| format!("{x:02x}")).collect();
        let path = self.directory.join(name);
        if path.exists() {
            if fs::read(&path).map_err(|_| "read")? != canonical {
                return Err("stored corruption");
            }
            File::open(&path)
                .and_then(|f| f.sync_all())
                .map_err(|_| "file sync")?;
            File::open(&self.directory)
                .and_then(|f| f.sync_all())
                .map_err(|_| "directory sync")?;
        } else {
            atomic_save(&path, &canonical, fault)?;
        }
        Ok(id)
    }
    pub fn pending(&self) -> Vec<Vec<u8>> {
        fs::read_dir(&self.directory)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().len() == 64)
            .map(|e| fs::read(e.path()).unwrap())
            .collect()
    }
}

/// Persist document and applied IDs in the same encrypted checkpoint before success.
pub fn apply_pending(
    inbox: &LockedInbox,
    snapshot: &mut Snapshot,
    path: &Path,
    password: &[u8],
    fault: Fault,
) -> Result<usize> {
    use automerge::{Automerge, ReadDoc};
    if snapshot.group != inbox.group || snapshot.epoch != inbox.epoch {
        return Err("unlock current epoch required");
    }
    let mut doc = Automerge::load(&snapshot.document).map_err(|_| "document")?;
    let mut applied = snapshot.applied.clone();
    let mut pending = Vec::new();
    for bytes in inbox.pending() {
        let id = digest(&bytes);
        if applied.contains(&id) {
            continue;
        }
        // Recheck CURRENT admission after unlock: previously received is not applied.
        let Ok(e) = inbox.validate(&bytes) else {
            continue;
        };
        let clear = e.decrypt(&snapshot.read_key)?;
        let change = automerge::Change::from_bytes(clear.to_vec()).map_err(|_| "change")?;
        // Bind Automerge actor identity to authenticated author.
        if change.actor_id().to_bytes() != e.author {
            return Err("actor identity");
        }
        pending.push((id, change));
    }
    let before = applied.len();
    loop {
        let n = pending.len();
        let mut deferred = Vec::new();
        for (id, change) in pending {
            if change
                .deps()
                .iter()
                .all(|h| doc.get_change_by_hash(h).is_some())
            {
                doc.apply_changes([change]).map_err(|_| "apply")?;
                applied.insert(id);
            } else {
                deferred.push((id, change));
            }
        }
        pending = deferred;
        if pending.is_empty() || pending.len() == n {
            break;
        }
    }
    let next = Snapshot {
        group: snapshot.group,
        epoch: snapshot.epoch,
        read_key: snapshot.read_key,
        document: doc.save(),
        applied,
    };
    atomic_save(path, &seal(&next, password)?, fault)?;
    let count = next.applied.len() - before;
    *snapshot = next;
    Ok(count)
}

/// Control proof validates a linear history from a pinned trust root, not global consensus.
#[derive(Clone, Serialize, Deserialize)]
pub struct Control {
    pub group: [u8; 16],
    pub seq: u64,
    pub epoch: u64,
    pub manager: Id,
    pub members: BTreeSet<Id>,
    pub previous: Id,
}
impl Control {
    pub fn hash(&self) -> Id {
        digest(&serde_json::to_vec(self).unwrap())
    }
    pub fn sign_successor(
        &self,
        next: &Control,
        key: &SigningKey,
        unlocked: bool,
    ) -> Result<Vec<u8>> {
        if !unlocked || key.verifying_key().to_bytes() != self.manager {
            return Err("no management authority");
        }
        let mut bytes = b"Pass2P/spike/control/1".to_vec();
        bytes.extend(serde_json::to_vec(next).unwrap());
        Ok(key.sign(&bytes).to_bytes().to_vec())
    }
    pub fn accept(&self, next: &Control, signature: &[u8]) -> Result<()> {
        if next.group != self.group
            || next.seq != self.seq + 1
            || next.epoch < self.epoch
            || next.previous != self.hash()
            || !next.members.contains(&next.manager)
        {
            return Err("control continuity");
        }
        if self.members.difference(&next.members).next().is_some() && next.epoch <= self.epoch {
            return Err("revoke requires rotation");
        }
        let mut bytes = b"Pass2P/spike/control/1".to_vec();
        bytes.extend(serde_json::to_vec(next).unwrap());
        VerifyingKey::from_bytes(&self.manager)
            .map_err(|_| "manager")?
            .verify_strict(
                &bytes,
                &Signature::from_slice(signature).map_err(|_| "signature")?,
            )
            .map_err(|_| "control signature")
    }
}

/// One manager process, synthetic clock and persistent consumed-token journal.
#[derive(Serialize, Deserialize)]
pub struct Invitation {
    pub token_hash: Id,
    pub expires: u64,
    pub consumed: bool,
}
impl Invitation {
    pub fn redeem(
        &mut self,
        token: &[u8],
        now: u64,
        approved: bool,
        path: &Path,
        fault: Fault,
    ) -> Result<()> {
        if self.consumed || now >= self.expires || digest(token) != self.token_hash {
            return Err("invitation invalid");
        }
        // Denial consumes it too. Failed durable write yields no admission.
        let next = Self {
            token_hash: self.token_hash,
            expires: self.expires,
            consumed: true,
        };
        atomic_save(path, &serde_json::to_vec(&next).unwrap(), fault)?;
        self.consumed = true;
        if !approved {
            return Err("manager denied");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests;

#[cfg(all(target_os = "android", feature = "android-jni"))]
mod android_jni;
#[cfg(feature = "network")]
pub mod network;
