use crate::{Error, MAX_FILE_SIZE};
use argon2::{Algorithm, Argon2, Params, Version};
use chacha20poly1305::{
    XChaCha20Poly1305, XNonce,
    aead::{Aead, KeyInit, Payload},
};
use rand_core::{OsRng, RngCore};
use std::io::{Read, Write};
use std::time::Instant;
use zeroize::Zeroizing;

const PREFIX: usize = 32;
const WRAPPED: usize = 104;
pub(super) const HEADER: usize = 136;
const CHUNK: usize = 1024 * 1024;
pub(super) const MAX_PAYLOAD: u64 = 16 * 1024 * 1024 * 1024;
pub(super) const MAX_ENCODED_SIZE: u64 =
    HEADER as u64 + MAX_PAYLOAD + (MAX_PAYLOAD.div_ceil(CHUNK as u64) + 1) * 40;
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
pub(super) fn validate(header: &[u8], file_length: u64) -> Result<(), Error> {
    if header.len() != HEADER || &header[..8] != MAGIC {
        return Err(Error::InvalidFile);
    }
    if header[8..12] != [1, 0, 2, 0] {
        return Err(Error::UnsupportedVersion);
    }
    let t = iterations(header)?;
    if !(3..=256).contains(&t) {
        return Err(Error::InvalidFile);
    }
    let length = payload_length(header)?;
    if length > MAX_PAYLOAD {
        return Err(Error::TooLarge);
    }
    let frames = length.div_ceil(CHUNK as u64) + 1;
    let expected = HEADER as u64 + length + frames * 40;
    if expected != file_length {
        return Err(Error::InvalidFile);
    }
    Ok(())
}

fn iterations(header: &[u8]) -> Result<u32, Error> {
    Ok(u32::from_le_bytes(
        header[12..16].try_into().map_err(|_| Error::InvalidFile)?,
    ))
}

pub(super) fn payload_length(header: &[u8]) -> Result<u64, Error> {
    if header.len() != HEADER {
        return Err(Error::InvalidFile);
    }
    Ok(u64::from_le_bytes(
        header[128..136]
            .try_into()
            .map_err(|_| Error::InvalidFile)?,
    ))
}

pub(super) fn create_header(password: &[u8], target_ms: u32) -> Result<(Vec<u8>, ReadKey), Error> {
    if !(500..=5000).contains(&target_ms) {
        return Err(Error::InvalidFile);
    }
    let salt = random::<16>()?;
    let start = Instant::now();
    let trial = derive(password, &salt, 3)?;
    let elapsed = start.elapsed().as_millis().max(1);
    let iterations = (u128::from(target_ms) * 3 / elapsed).clamp(3, 256) as u32;
    let kek = if iterations == 3 {
        trial
    } else {
        derive(password, &salt, iterations)?
    };
    let key = ReadKey(Zeroizing::new(random()?));
    let mut bytes = Vec::from(MAGIC.as_slice());
    bytes.extend([1, 0, 2, 0]);
    bytes.extend(iterations.to_le_bytes());
    bytes.extend(salt);
    let nonce = random::<24>()?;
    let wrapped = seal(&kek, &nonce, &aad(&bytes, b"/wrap"), key.0.as_ref())?;
    bytes.extend(nonce);
    bytes.extend(wrapped);
    bytes.resize(HEADER, 0);
    Ok((bytes, key))
}

pub(super) fn unlock_key(header: &[u8], password: &[u8]) -> Result<ReadKey, Error> {
    let kek = derive(password, &header[16..32], iterations(header)?)?;
    let raw = open(
        &kek,
        &header[PREFIX..56],
        &aad(&header[..PREFIX], b"/wrap"),
        &header[56..WRAPPED],
    )?;
    let mut key = ReadKey(Zeroizing::new([0; 32]));
    if raw.len() != 32 {
        return Err(Error::InvalidFile);
    }
    key.0.copy_from_slice(&raw);
    Ok(key)
}

fn chunk_aad(header: &[u8], index: u64, final_chunk: bool) -> Vec<u8> {
    let mut data = aad(header, b"/chunk/v1");
    data.extend(index.to_le_bytes());
    data.push(u8::from(final_chunk));
    data
}

pub(super) fn encrypt_stream(
    header: &[u8],
    key: &ReadKey,
    mut clear: impl Read,
    length: u64,
    writer: &mut impl Write,
) -> Result<Vec<u8>, Error> {
    if length > MAX_PAYLOAD {
        return Err(Error::TooLarge);
    }
    let mut next = header[..WRAPPED].to_vec();
    next.extend(random::<24>()?);
    next.extend(length.to_le_bytes());
    writer.write_all(&next)?;
    let mut remaining = length;
    let mut index = 0;
    let mut buffer = Zeroizing::new(vec![0; CHUNK]);
    loop {
        let final_chunk = remaining == 0;
        let size = remaining.min(CHUNK as u64) as usize;
        clear.read_exact(&mut buffer[..size])?;
        let nonce = random::<24>()?;
        let cipher = seal(
            key,
            &nonce,
            &chunk_aad(&next, index, final_chunk),
            &buffer[..size],
        )?;
        writer.write_all(&nonce)?;
        writer.write_all(&cipher)?;
        if final_chunk {
            break;
        }
        remaining -= size as u64;
        index += 1;
    }
    let mut extra = [0];
    if clear.read(&mut extra)? != 0 {
        return Err(Error::InvalidFile);
    }
    Ok(next)
}

pub(super) fn decrypt_stream(
    header: &[u8],
    key: &ReadKey,
    reader: &mut impl Read,
    writer: &mut impl Write,
) -> Result<(), Error> {
    let mut remaining = payload_length(header)?;
    let mut index = 0;
    loop {
        let final_chunk = remaining == 0;
        let size = remaining.min(CHUNK as u64) as usize;
        let mut nonce = [0; 24];
        reader.read_exact(&mut nonce)?;
        let mut cipher = vec![0; size + 16];
        reader.read_exact(&mut cipher)?;
        let clear = open(key, &nonce, &chunk_aad(header, index, final_chunk), &cipher)?;
        writer.write_all(&clear)?;
        if final_chunk {
            break;
        }
        remaining -= size as u64;
        index += 1;
    }
    let mut extra = [0];
    if reader.read(&mut extra)? != 0 {
        return Err(Error::InvalidFile);
    }
    Ok(())
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
