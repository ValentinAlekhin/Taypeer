//! One bounded, epoch-authenticated credential format for file and isolated workers.
use super::{CREDENTIAL_MAGIC, LocalStateKey};
use crate::{Error, ReadKey, crypto};
use serde::{Deserialize, Serialize};
use std::io::Cursor;
use taypeer_core::DatabaseId;
use taypeer_trust::{AuthorKey, Digest, TransportKey};
use zeroize::Zeroizing;

const VERSION: u16 = 1;
const MAX_CLEAR: usize = 64 * 1024;
const MAX_ENCODED: usize = crypto::HEADER + MAX_CLEAR + 80;
const JOIN_MAGIC: &[u8; 8] = b"TAYJON1\0";

/// Public identity of one local credential, independent of the host operating system.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LocalCredentialScope {
    /// Logical database identity; copying its portable file does not enroll a device.
    pub database: DatabaseId,
    /// Genesis control hash, separating independently recovered trusted sets.
    pub root: Digest,
    /// Database key epoch which protects this record.
    pub epoch: u64,
}

/// Worker-owned local authority. No Debug, Clone or plaintext serialization API.
/// The host may persist its ciphertext but must never receive this decrypted value.
pub struct LocalCredential {
    scope: LocalCredentialScope,
    author: Option<Zeroizing<[u8; 32]>>,
    transport: Zeroizing<[u8; 32]>,
    local_state: Zeroizing<[u8; 32]>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct Record {
    version: u16,
    database: DatabaseId,
    root: Digest,
    epoch: u64,
    author: Option<Zeroizing<[u8; 32]>>,
    transport: Zeroizing<[u8; 32]>,
    local_state: Zeroizing<[u8; 32]>,
}

impl LocalCredential {
    /// Build authority for explicit creation, invitation enrollment or trust recovery.
    /// Callers must first authenticate the database and authorize enrollment.
    pub fn new(
        scope: LocalCredentialScope,
        author: &AuthorKey,
        transport: &TransportKey,
        local: &LocalStateKey,
    ) -> Self {
        Self {
            scope,
            author: Some(author.secret_seed()),
            transport: transport.secret_seed(),
            local_state: Zeroizing::new(*local.secret_bytes()),
        }
    }

    /// Protect local drafts, access marks and recovery receipts for an imported file.
    /// Its independent transport identity is not admitted to any network trust set.
    pub fn read_only(
        scope: LocalCredentialScope,
        transport: &TransportKey,
        local: &LocalStateKey,
    ) -> Self {
        Self {
            scope,
            author: None,
            transport: transport.secret_seed(),
            local_state: Zeroizing::new(*local.secret_bytes()),
        }
    }

    /// Exact authenticated binding; does not grant network admission by itself.
    pub fn scope(&self) -> &LocalCredentialScope {
        &self.scope
    }

    /// Reconstruct the author only inside an authenticated worker.
    pub fn author(&self) -> Option<AuthorKey> {
        self.author.as_ref().map(|seed| AuthorKey::from_seed(seed))
    }

    /// Reconstruct ciphertext transport authority, independently of author access.
    pub fn transport(&self) -> TransportKey {
        TransportKey::from_seed(&self.transport)
    }

    /// Reconstruct the independent key protecting this device's state.
    pub fn local_state_key(&self) -> LocalStateKey {
        LocalStateKey::from_secret(&self.local_state)
    }

    /// Advance the binding only after a verified rotation; seal with the new epoch key.
    pub fn set_epoch(&mut self, epoch: u64) {
        self.scope.epoch = epoch;
    }

    /// Encrypt a bounded record with the existing TAYCRD1 authenticated stream.
    /// Returned bytes are ciphertext suitable for a host-owned atomic publication.
    pub fn seal(&self, epoch: &ReadKey) -> Result<Vec<u8>, Error> {
        let clear = self.clear()?;
        let header = crypto::keyed_header(CREDENTIAL_MAGIC)?;
        let mut cipher = Vec::new();
        crypto::encrypt_stream(
            &header,
            epoch,
            clear.as_slice(),
            clear.len() as u64,
            &mut cipher,
        )?;
        Ok(cipher)
    }

    /// Protect pending invitation keys before the received database epoch is known.
    /// This local-only domain uses the existing Argon2id password wrapper and stream.
    pub fn seal_with_password(&self, password: &[u8], target_ms: u32) -> Result<Vec<u8>, Error> {
        let clear = self.clear()?;
        let (header, key) = crypto::create_header_with_magic(password, target_ms, JOIN_MAGIC)?;
        let mut cipher = Vec::new();
        crypto::encrypt_stream(
            &header,
            &key,
            clear.as_slice(),
            clear.len() as u64,
            &mut cipher,
        )?;
        Ok(cipher)
    }

    /// Resume encrypted invitation staging with its original password and exact scope.
    /// A wrong password or substituted record never falls back to a new identity.
    pub fn open_with_password(
        cipher: &[u8],
        password: &[u8],
        expected: &LocalCredentialScope,
    ) -> Result<Self, Error> {
        let header = bounded_header(cipher, JOIN_MAGIC)?;
        let key = crypto::unlock_key_with_magic(header, password, JOIN_MAGIC)?;
        Self::decode(cipher, &key, header, expected)
    }

    /// Authenticate complete ciphertext and its expected scope before returning authority.
    /// Wrong keys, substituted roots/epochs and truncated or trailing bytes are rejected.
    pub fn open(
        cipher: &[u8],
        epoch: &ReadKey,
        expected: &LocalCredentialScope,
    ) -> Result<Self, Error> {
        let header = bounded_header(cipher, CREDENTIAL_MAGIC)?;
        Self::decode(cipher, epoch, header, expected)
    }

    fn decode(
        cipher: &[u8],
        key: &ReadKey,
        header: &[u8],
        expected: &LocalCredentialScope,
    ) -> Result<Self, Error> {
        let mut input = Cursor::new(cipher);
        input.set_position(crypto::HEADER as u64);
        let mut clear = Zeroizing::new(Vec::new());
        crypto::decrypt_stream(header, key, &mut input, &mut *clear)?;
        Self::from_clear(&clear, expected)
    }

    pub(super) fn clear(&self) -> Result<Zeroizing<Vec<u8>>, Error> {
        let record = Record {
            version: VERSION,
            database: self.scope.database.clone(),
            root: self.scope.root,
            epoch: self.scope.epoch,
            author: self.author.clone(),
            transport: self.transport.clone(),
            local_state: self.local_state.clone(),
        };
        let clear = Zeroizing::new(serde_json::to_vec(&record).map_err(|_| Error::InvalidFile)?);
        if clear.len() > MAX_CLEAR {
            return Err(Error::TooLarge);
        }
        Ok(clear)
    }

    pub(super) fn from_clear(clear: &[u8], expected: &LocalCredentialScope) -> Result<Self, Error> {
        if clear.len() > MAX_CLEAR {
            return Err(Error::TooLarge);
        }
        let record: Record = serde_json::from_slice(clear).map_err(|_| Error::InvalidFile)?;
        if record.version != VERSION {
            return Err(Error::UnsupportedVersion);
        }
        let scope = LocalCredentialScope {
            database: record.database,
            root: record.root,
            epoch: record.epoch,
        };
        if scope != *expected {
            return Err(Error::Authentication);
        }
        Ok(Self {
            scope,
            author: record.author,
            transport: record.transport,
            local_state: record.local_state,
        })
    }
}

fn bounded_header<'a>(cipher: &'a [u8], magic: &[u8; 8]) -> Result<&'a [u8], Error> {
    if cipher.len() > MAX_ENCODED {
        return Err(Error::TooLarge);
    }
    let header = cipher.get(..crypto::HEADER).ok_or(Error::InvalidFile)?;
    crypto::validate_keyed(header, cipher.len() as u64, magic)?;
    if crypto::payload_length(header)? > MAX_CLEAR as u64 {
        return Err(Error::TooLarge);
    }
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LocalCredentialStore, LocalStateStore};

    fn fixture() -> (LocalCredential, ReadKey) {
        let scope = LocalCredentialScope {
            database: DatabaseId::new("PUBLIC-local-credential"),
            root: Digest::of(b"PUBLIC-root"),
            epoch: 2,
        };
        let record = LocalCredential::new(
            scope,
            &AuthorKey::from_seed(&[17; 32]),
            &TransportKey::from_seed(&[18; 32]),
            &LocalStateKey::from_secret(&[19; 32]),
        );
        (record, ReadKey::from_secret(&[20; 32]))
    }

    #[test]
    fn ciphertext_codec_matches_file_store_and_retains_exact_authority() {
        let (record, key) = fixture();
        let cipher = record.seal(&key).unwrap();
        let opened = LocalCredential::open(&cipher, &key, record.scope()).unwrap();
        assert_eq!(
            opened.author().unwrap().public(),
            record.author().unwrap().public()
        );
        assert_eq!(opened.transport().public(), record.transport().public());
        assert_eq!(opened.local_state_key().secret_bytes(), &[19; 32]);

        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC credentials");
        std::fs::write(&path, cipher).unwrap();
        let store = LocalCredentialStore::open(&path).unwrap();
        assert_eq!(
            store
                .unlock_record(&key, record.scope())
                .unwrap()
                .author()
                .unwrap()
                .public(),
            record.author().unwrap().public()
        );
        drop(store);
        assert!(LocalStateStore::open(&path).is_err());
    }

    #[test]
    fn read_only_envelope_retains_local_state_without_author_or_admission() {
        let (enrolled, key) = fixture();
        let reader = LocalCredential::read_only(
            enrolled.scope().clone(),
            &enrolled.transport(),
            &enrolled.local_state_key(),
        );
        let cipher = reader.seal(&key).unwrap();
        let opened = LocalCredential::open(&cipher, &key, reader.scope()).unwrap();
        assert!(opened.author().is_none());
        assert_eq!(opened.transport().public(), enrolled.transport().public());
        assert_eq!(opened.local_state_key().secret_bytes(), &[19; 32]);
    }

    #[test]
    fn invitation_staging_authenticates_password_scope_and_domain() {
        let (record, key) = fixture();
        let cipher = record
            .seal_with_password(b"PUBLIC staging password", 500)
            .unwrap();
        let opened = LocalCredential::open_with_password(
            &cipher,
            b"PUBLIC staging password",
            record.scope(),
        )
        .unwrap();
        assert_eq!(
            opened.author().unwrap().public(),
            record.author().unwrap().public()
        );
        assert!(
            LocalCredential::open_with_password(&cipher, b"PUBLIC wrong password", record.scope())
                .is_err()
        );
        assert!(LocalCredential::open(&cipher, &key, record.scope()).is_err());
        assert!(
            LocalCredential::open_with_password(
                &record.seal(&key).unwrap(),
                b"PUBLIC staging password",
                record.scope()
            )
            .is_err()
        );
        let wrong = LocalCredentialScope {
            root: Digest::of(b"PUBLIC wrong staging root"),
            ..record.scope().clone()
        };
        assert!(matches!(
            LocalCredential::open_with_password(&cipher, b"PUBLIC staging password", &wrong),
            Err(Error::Authentication)
        ));
    }

    #[test]
    fn codec_rejects_wrong_key_scope_tampering_and_extra_bytes() {
        let (record, key) = fixture();
        let cipher = record.seal(&key).unwrap();
        assert!(
            LocalCredential::open(&cipher, &ReadKey::from_secret(&[21; 32]), record.scope())
                .is_err()
        );
        for scope in [
            LocalCredentialScope {
                root: Digest::of(b"PUBLIC other root"),
                ..record.scope().clone()
            },
            LocalCredentialScope {
                epoch: 3,
                ..record.scope().clone()
            },
            LocalCredentialScope {
                database: DatabaseId::new("PUBLIC other database"),
                ..record.scope().clone()
            },
        ] {
            assert!(matches!(
                LocalCredential::open(&cipher, &key, &scope),
                Err(Error::Authentication)
            ));
        }
        let mut tampered = cipher.clone();
        *tampered.last_mut().unwrap() ^= 1;
        assert!(LocalCredential::open(&tampered, &key, record.scope()).is_err());
        let mut extended = cipher.clone();
        extended.push(0);
        assert!(LocalCredential::open(&extended, &key, record.scope()).is_err());
        assert!(LocalCredential::open(&cipher[..cipher.len() - 1], &key, record.scope()).is_err());
    }

    #[test]
    fn typed_file_rotation_changes_key_and_scope_together() {
        let (mut record, old) = fixture();
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC credentials");
        let mut store = LocalCredentialStore::create_record(&path, &old, &record).unwrap();
        let new = ReadKey::from_secret(&[22; 32]);
        let previous = record.scope().clone();
        record.set_epoch(3);
        store.save_record(&new, &record).unwrap();
        drop(store);
        let store = LocalCredentialStore::open(&path).unwrap();
        assert!(store.unlock_record(&old, &previous).is_err());
        assert!(store.unlock_record(&new, &previous).is_err());
        assert_eq!(
            store.unlock_record(&new, record.scope()).unwrap().scope(),
            record.scope()
        );
        let bytes = std::fs::read(&path).unwrap();
        assert!(LocalCredential::open(&bytes, &new, record.scope()).is_ok());
    }
}
