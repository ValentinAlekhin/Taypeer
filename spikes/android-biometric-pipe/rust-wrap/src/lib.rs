//! Isolated PUBLIC interoperability fixture; this crate never accepts a database key.
use rand_core::OsRng;
use rsa::{BigUint, Oaep, RsaPublicKey, pkcs8::DecodePublicKey, traits::PublicKeyParts};
use sha1::Sha1;
use sha2::Sha256;

/// Maximum DER SPKI size accepted from the PUBLIC Android spike.
pub const SPKI_LIMIT: usize = 4 * 1024;
/// RSA-2048 produces exactly this many ciphertext bytes.
pub const CIPHERTEXT_LENGTH: usize = 256;
/// PUBLIC, nonsecret plaintext expected by the separate Android JNI pipe spike.
pub const PUBLIC_FIXTURE: [u8; 32] = [0x50; 32];

/// Content-free refusal categories; DER, key material and filesystem paths are not logged.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum WrapError {
    /// Empty or oversized DER was refused before parsing.
    InputLength,
    /// Input is not one exact RSA DER SubjectPublicKeyInfo.
    Spki,
    /// This probe supports exactly RSA-2048 with the F4 exponent.
    Parameters,
    /// RSA/OAEP encryption did not produce its expected bounded result.
    Encryption,
}

/// Wrap only the fixed PUBLIC fixture using RSA-OAEP SHA-256 / MGF1 SHA-1 / empty label.
/// Input must be a single DER SPKI, at most 4 KiB, containing exactly RSA-2048/F4.
/// There is deliberately no parameter for a ReadKey, password or arbitrary plaintext.
pub fn wrap_public_fixture(spki_der: &[u8]) -> Result<[u8; CIPHERTEXT_LENGTH], WrapError> {
    if spki_der.is_empty() || spki_der.len() > SPKI_LIMIT {
        return Err(WrapError::InputLength);
    }
    let key = RsaPublicKey::from_public_key_der(spki_der).map_err(|_| WrapError::Spki)?;
    if key.n().bits() != 2048 || key.e() != &BigUint::from(65537u32) {
        return Err(WrapError::Parameters);
    }
    // The pinned API explicitly selects an empty label and separate main/MGF digests.
    let ciphertext = key
        .encrypt(
            &mut OsRng,
            Oaep::new_with_mgf_hash::<Sha256, Sha1>(),
            &PUBLIC_FIXTURE,
        )
        .map_err(|_| WrapError::Encryption)?;
    ciphertext.try_into().map_err(|_| WrapError::Encryption)
}

#[cfg(test)]
mod tests {
    use super::*;
    use rsa::{RsaPrivateKey, pkcs1::EncodeRsaPublicKey, pkcs8::EncodePublicKey};
    use std::sync::OnceLock;

    fn public_test_private_key() -> &'static RsaPrivateKey {
        static KEY: OnceLock<RsaPrivateKey> = OnceLock::new();
        // Deliberately ephemeral PUBLIC software fixture; never persisted or used by Taypeer.
        KEY.get_or_init(|| RsaPrivateKey::new(&mut OsRng, 2048).unwrap())
    }

    fn spki(key: &RsaPublicKey) -> Vec<u8> {
        key.to_public_key_der().unwrap().as_bytes().to_vec()
    }

    #[test]
    fn public_fixture_round_trips_with_exact_android_oaep_parameters() {
        let private = public_test_private_key();
        let ciphertext = wrap_public_fixture(&spki(&private.to_public_key())).unwrap();
        assert_eq!(ciphertext.len(), 256);
        assert_eq!(
            private
                .decrypt(Oaep::new_with_mgf_hash::<Sha256, Sha1>(), &ciphertext)
                .unwrap(),
            PUBLIC_FIXTURE
        );
        assert!(private.decrypt(Oaep::new::<Sha256>(), &ciphertext).is_err());
        assert!(private.decrypt(Oaep::new::<Sha1>(), &ciphertext).is_err());
        assert!(
            private
                .decrypt(
                    Oaep::new_with_mgf_hash_and_label::<Sha256, Sha1, _>("PUBLIC wrong label"),
                    &ciphertext
                )
                .is_err()
        );
    }

    #[test]
    fn oaep_randomizes_two_wraps_of_the_same_public_fixture() {
        let der = spki(&public_test_private_key().to_public_key());
        let first = wrap_public_fixture(&der).unwrap();
        let second = wrap_public_fixture(&der).unwrap();
        assert_ne!(first, second);
        for ciphertext in [first, second] {
            assert_eq!(
                public_test_private_key()
                    .decrypt(Oaep::new_with_mgf_hash::<Sha256, Sha1>(), &ciphertext)
                    .unwrap(),
                PUBLIC_FIXTURE
            );
        }
    }

    #[test]
    fn empty_and_overlength_input_refuse_before_der_parsing() {
        assert_eq!(wrap_public_fixture(&[]), Err(WrapError::InputLength));
        assert_eq!(
            wrap_public_fixture(&vec![0x50; SPKI_LIMIT + 1]),
            Err(WrapError::InputLength)
        );
    }

    #[test]
    fn malformed_truncated_and_trailing_der_refuse_exact_format() {
        let der = spki(&public_test_private_key().to_public_key());
        for input in [
            vec![0x50; 32],
            der[..der.len() - 1].to_vec(),
            [der.as_slice(), &[0x00]].concat(),
        ] {
            assert_eq!(wrap_public_fixture(&input), Err(WrapError::Spki));
        }
    }

    #[test]
    fn pkcs1_is_not_accepted_as_subject_public_key_info() {
        let der = public_test_private_key()
            .to_public_key()
            .to_pkcs1_der()
            .unwrap();
        assert_eq!(wrap_public_fixture(der.as_bytes()), Err(WrapError::Spki));
    }

    #[test]
    fn other_valid_rsa_exponent_is_refused() {
        let key =
            RsaPublicKey::new(public_test_private_key().n().clone(), BigUint::from(3u8)).unwrap();
        assert_eq!(wrap_public_fixture(&spki(&key)), Err(WrapError::Parameters));
    }

    #[test]
    fn only_exact_2048_bit_modulus_is_accepted() {
        for bits in [1024usize, 2047, 2049, 3072] {
            let modulus = (BigUint::from(1u8) << (bits - 1)) + BigUint::from(3u8);
            let key = RsaPublicKey::new(modulus, BigUint::from(65537u32)).unwrap();
            assert_eq!(wrap_public_fixture(&spki(&key)), Err(WrapError::Parameters));
        }
    }
}
