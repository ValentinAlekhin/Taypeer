//! The isolated worker wraps exactly one current key using the pinned OS-compatible scheme.
use crate::RuntimeError;
use rsa::{
    Oaep, RsaPublicKey,
    pkcs8::{DecodePublicKey, EncodePublicKey},
    traits::PublicKeyParts,
};
use taypeer_storage::ReadKey;
use taypeer_trust::Digest;

const MAX_SPKI: usize = 4096;
const RSA_BYTES: usize = 256;

/// Validate the fixed platform owner's bounded canonical RSA2048/F4 DER SPKI.
/// Returns its public digest only; this validates key shape, never OS policy,
/// authentication, database authority or a right to wrap a ReadKey.
pub fn validate_biometric_spki(spki: &[u8]) -> Result<Digest, RuntimeError> {
    let _ = public_key(spki)?;
    Ok(Digest::of(spki))
}

fn public_key(spki: &[u8]) -> Result<RsaPublicKey, RuntimeError> {
    if spki.is_empty() || spki.len() > MAX_SPKI {
        return Err(RuntimeError::Protocol);
    }
    let public = RsaPublicKey::from_public_key_der(spki).map_err(|_| RuntimeError::Protocol)?;
    if public.n().bits() != 2048
        || public.e() != &rsa::BigUint::from(65537u32)
        || public
            .to_public_key_der()
            .map_err(|_| RuntimeError::Protocol)?
            .as_bytes()
            != spki
    {
        return Err(RuntimeError::Protocol);
    }
    Ok(public)
}

pub(super) fn wrap(key: &ReadKey, spki: &[u8]) -> Result<Vec<u8>, RuntimeError> {
    let public = public_key(spki)?;
    let ciphertext = public
        .encrypt(
            &mut rand_core::OsRng,
            Oaep::new_with_mgf_hash::<sha2::Sha256, sha1::Sha1>(),
            key.secret_bytes(),
        )
        .map_err(|_| RuntimeError::Protocol)?;
    if ciphertext.len() != RSA_BYTES {
        return Err(RuntimeError::Protocol);
    }
    Ok(ciphertext)
}
