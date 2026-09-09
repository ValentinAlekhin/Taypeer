use crate::{Error, MAX_FILE_SIZE};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand_core::{OsRng, RngCore};
use std::time::Instant;
use zeroize::Zeroizing;

const PREFIX: usize = 32;
const WRAPPED: usize = 104;
const HEADER: usize = 128;
const MAGIC: &[u8; 8] = b"TAYPEER\0";

/// An unlocked content key. Not cloneable or printable; its own allocation is zeroized.
/// This does not guarantee erasure of plaintext in other allocations.
pub struct ReadKey(Zeroizing<[u8; 32]>);

fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| Error::Random)?;
    Ok(bytes)
}
fn derive(password: &[u8], salt: &[u8], iterations: u32) -> Result<ReadKey, Error> {
    if password.is_empty() {
        return Err(Error::EmptyPassword);
    }
    let params = Params::new(64 * 1024, iterations, 1, Some(32)).map_err(|_| Error::InvalidFile)?;
    let mut key = Zeroizing::new([0; 32]);
    Argon2::new(Algorithm::Argon2id, Version::V0x13, params)
        .hash_password_into(password, salt, key.as_mut())
        .map_err(|_| Error::InvalidFile)?;
    Ok(ReadKey(key))
}
fn seal(key: &ReadKey, nonce: &[u8], aad: &[u8], clear: &[u8]) -> Result<Vec<u8>, Error> {
    XChaCha20Poly1305::new_from_slice(key.0.as_ref())
        .map_err(|_| Error::InvalidFile)?
        .encrypt(XNonce::from_slice(nonce), Payload { msg: clear, aad })
        .map_err(|_| Error::TooLarge)
}
fn open(
    key: &ReadKey,
    nonce: &[u8],
    aad: &[u8],
    cipher: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    XChaCha20Poly1305::new_from_slice(key.0.as_ref())
        .map_err(|_| Error::InvalidFile)?
        .decrypt(XNonce::from_slice(nonce), Payload { msg: cipher, aad })
        .map(Zeroizing::new)
        .map_err(|_| Error::Authentication)
}
fn aad(header: &[u8], domain: &[u8]) -> Vec<u8> {
    [header, domain].concat()
}
pub(super) fn validate(bytes: &[u8]) -> Result<(), Error> {
    if bytes.len() > MAX_FILE_SIZE {
        return Err(Error::TooLarge);
    }
    if bytes.len() < HEADER + 16 || &bytes[..8] != MAGIC {
        return Err(Error::InvalidFile);
    }
    if bytes[8..12] != [0, 0, 1, 0] {
        return Err(Error::UnsupportedVersion);
    }
    let t = u32::from_le_bytes(bytes[12..16].try_into().map_err(|_| Error::InvalidFile)?);
    if !(3..=32).contains(&t) {
        return Err(Error::InvalidFile);
    }
    Ok(())
}
pub(super) fn create(password: &[u8], clear: &[u8]) -> Result<(Vec<u8>, ReadKey), Error> {
    let salt = random::<16>()?;
    // Measure actual work, then choose bounded parameters for the one-second target.
    let start = Instant::now();
    let trial = derive(password, &salt, 3)?;
    let elapsed = start.elapsed().as_millis().max(1);
    let iterations = (3000 / elapsed).clamp(3, 32) as u32;
    let kek = if iterations == 3 {
        trial
    } else {
        derive(password, &salt, iterations)?
    };
    let key = ReadKey(Zeroizing::new(random()?));
    let mut bytes = Vec::from(MAGIC.as_slice());
    bytes.extend([0, 0, 1, 0]);
    bytes.extend(iterations.to_le_bytes());
    bytes.extend(salt);
    let nonce = random::<24>()?;
    let wrapped = seal(&kek, &nonce, &aad(&bytes, b"/wrap"), key.0.as_ref())?;
    bytes.extend(nonce);
    bytes.extend(wrapped);
    bytes.resize(HEADER, 0);
    Ok((encrypt(&bytes, &key, clear)?, key))
}
pub(super) fn unlock(
    bytes: &[u8],
    password: &[u8],
) -> Result<(ReadKey, Zeroizing<Vec<u8>>), Error> {
    validate(bytes)?;
    let iterations = u32::from_le_bytes(bytes[12..16].try_into().map_err(|_| Error::InvalidFile)?);
    let kek = derive(password, &bytes[16..32], iterations)?;
    let raw = open(
        &kek,
        &bytes[PREFIX..56],
        &aad(&bytes[..PREFIX], b"/wrap"),
        &bytes[56..WRAPPED],
    )?;
    let mut key = ReadKey(Zeroizing::new([0; 32]));
    if raw.len() != 32 {
        return Err(Error::InvalidFile);
    }
    key.0.copy_from_slice(&raw);
    let clear = open(
        &key,
        &bytes[WRAPPED..HEADER],
        &aad(&bytes[..HEADER], b"/content"),
        &bytes[HEADER..],
    )?;
    Ok((key, clear))
}
pub(super) fn encrypt(header: &[u8], key: &ReadKey, clear: &[u8]) -> Result<Vec<u8>, Error> {
    if clear.len() > MAX_FILE_SIZE - HEADER - 16 {
        return Err(Error::TooLarge);
    }
    let mut bytes = header[..WRAPPED].to_vec();
    let nonce = random::<24>()?;
    bytes.extend(nonce);
    let cipher = seal(key, &nonce, &aad(&bytes, b"/content"), clear)?;
    bytes.extend(cipher);
    Ok(bytes)
}
pub(super) fn seal_draft(header: &[u8], key: &ReadKey, clear: &[u8]) -> Result<Vec<u8>, Error> {
    if clear.len() > MAX_FILE_SIZE - 40 {
        return Err(Error::TooLarge);
    }
    let nonce = random::<24>()?;
    let mut bytes = nonce.to_vec();
    bytes.extend(seal(
        key,
        &nonce,
        &aad(&header[..WRAPPED], b"/local-draft"),
        clear,
    )?);
    Ok(bytes)
}
pub(super) fn open_draft(
    header: &[u8],
    key: &ReadKey,
    bytes: &[u8],
) -> Result<Zeroizing<Vec<u8>>, Error> {
    if bytes.len() < 40 {
        return Err(Error::InvalidFile);
    }
    open(
        key,
        &bytes[..24],
        &aad(&header[..WRAPPED], b"/local-draft"),
        &bytes[24..],
    )
}
