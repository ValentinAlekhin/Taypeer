//! PUBLIC password fixtures exercise actual Argon2id and authenticated stream bytes.
use super::*;
use std::time::Instant;

#[test]
fn measured_password_unlock_matches_the_encoded_header_and_authenticates_the_stream() {
    let password = b"PUBLIC measured KDF password";
    let (header, key) = crypto::create_header(password, 500).unwrap();
    let clear = b"PUBLIC KDF unchanged plaintext";
    let mut ciphertext = Vec::new();
    let streamed = crypto::encrypt_stream(
        &header,
        &key,
        clear.as_slice(),
        clear.len() as u64,
        &mut ciphertext,
    )
    .unwrap();
    crypto::validate(&streamed, ciphertext.len() as u64).unwrap();
    let before = ciphertext.clone();
    let (unlocked, measured) = crypto::unlock_key_measured(&streamed, password).unwrap();
    let iterations = u32::from_le_bytes(streamed[12..16].try_into().unwrap());
    assert_eq!(measured.parameters.iterations, iterations);
    assert_eq!(measured.parameters.memory_kib, 65_536);
    assert_eq!(measured.parameters.parallelism, 1);
    assert!(measured.elapsed_ms > 0);
    assert_eq!(
        password_kdf_parameters(&streamed).unwrap(),
        measured.parameters
    );
    let mut decoded = Vec::new();
    crypto::decrypt_stream(&streamed, &unlocked, &mut &ciphertext[136..], &mut decoded).unwrap();
    assert_eq!(decoded, clear);
    assert_eq!(ciphertext, before);
    assert!(password_key_binding_matches(&header, &streamed));
    assert!(matches!(
        crypto::unlock_key_measured(&streamed, b"PUBLIC wrong password"),
        Err(Error::Authentication)
    ));
    assert_eq!(ciphertext, before);
    let mut invalid = streamed.clone();
    invalid[12..16].copy_from_slice(&257_u32.to_le_bytes());
    assert!(matches!(
        password_kdf_parameters(&invalid),
        Err(Error::InvalidFile)
    ));
    assert!(matches!(
        crypto::unlock_key_measured(&invalid, password),
        Err(Error::InvalidFile)
    ));
}

#[test]
fn explicit_calibration_measures_a_bounded_preview_without_a_password_header() {
    for invalid in [0, 499, 5001, u32::MAX] {
        assert!(matches!(
            calibrate_password_kdf(invalid),
            Err(Error::InvalidFile)
        ));
    }
    let started = Instant::now();
    let preview = calibrate_password_kdf(500).unwrap();
    assert_eq!(preview.target_ms, 500);
    assert_eq!(preview.measurement.parameters.memory_kib, 65_536);
    assert_eq!(preview.measurement.parameters.parallelism, 1);
    assert!((3..=256).contains(&preview.measurement.parameters.iterations));
    assert!(preview.measurement.elapsed_ms > 0);
    assert!(u128::from(preview.measurement.elapsed_ms) <= started.elapsed().as_millis());
    assert!(password_kdf_parameters(b"PUBLIC non-header").is_err());
}
