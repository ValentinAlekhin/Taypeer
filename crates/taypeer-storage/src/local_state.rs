//! Local keyed containers reuse the authenticated FileStore stream and durable writer.
use crate::{Error, FileStore, ReadKey, crypto};
use std::path::Path;
use zeroize::Zeroizing;

const STATE_MAGIC: &[u8; 8] = b"TAYLOC1\0";
const CREDENTIAL_MAGIC: &[u8; 8] = b"TAYCRD1\0";

/// Independent random key for local transport, registration and checkpoint state.
/// Never derived from a password or sent with a portable database.
pub struct LocalStateKey(ReadKey);
impl LocalStateKey {
    /// Generate an independent key using the operating system's randomness source.
    pub fn generate() -> Result<Self, Error> {
        let bytes = Zeroizing::new(crypto::random::<32>()?);
        Ok(Self(ReadKey::from_secret(&bytes)))
    }
    /// Reconstitute a key obtained from authenticated local credentials or private IPC.
    pub fn from_secret(bytes: &[u8; 32]) -> Self {
        Self(ReadKey::from_secret(bytes))
    }
    /// Serialize only into encrypted credentials or a private transport capability.
    pub fn secret_bytes(&self) -> &[u8; 32] {
        self.0.secret_bytes()
    }
}

/// Encrypted per-database local state. Holds no plaintext or key between calls.
/// The exclusive lock, atomic replacement and fsync contract are those of FileStore.
pub struct LocalStateStore(FileStore);
impl LocalStateStore {
    /// Create a durable local state container without replacing an existing file.
    pub fn create(path: &Path, key: &LocalStateKey, clear: &[u8]) -> Result<Self, Error> {
        FileStore::create_keyed_object(path, &key.0, clear, STATE_MAGIC).map(Self)
    }
    /// Open an existing container without acquiring its key.
    pub fn open(path: &Path) -> Result<Self, Error> {
        FileStore::open_keyed_object(path, STATE_MAGIC).map(Self)
    }
    /// Authenticate a stable snapshot while the host retains the exclusive writer lock.
    /// Concurrent replacement returns Changed; this never grants write authority.
    pub fn read(path: &Path, key: &LocalStateKey) -> Result<Zeroizing<Vec<u8>>, Error> {
        FileStore::read_keyed_object(path, &key.0, STATE_MAGIC)
    }
    /// Authenticate the complete payload before exposing a zeroizing candidate.
    pub fn unlock(&self, key: &LocalStateKey) -> Result<Zeroizing<Vec<u8>>, Error> {
        self.0.unlock_keyed_object(&key.0)
    }
    /// Durably replace state; a directory fsync failure returns CommitUncertain.
    pub fn save(&mut self, key: &LocalStateKey, clear: &[u8]) -> Result<(), Error> {
        self.0.save(&key.0, clear)
    }
}

/// Local author credentials encrypted by a database epoch key.
/// This separate domain prevents a local state file being substituted for credentials.
pub struct LocalCredentialStore(FileStore);
impl LocalCredentialStore {
    /// Create credentials at an explicitly enrolled local path.
    pub fn create(path: &Path, epoch: &ReadKey, clear: &[u8]) -> Result<Self, Error> {
        FileStore::create_keyed_object(path, epoch, clear, CREDENTIAL_MAGIC).map(Self)
    }
    /// Open credentials, rejecting other file/container domains.
    pub fn open(path: &Path) -> Result<Self, Error> {
        FileStore::open_keyed_object(path, CREDENTIAL_MAGIC).map(Self)
    }
    /// Authenticate credentials with the unlocked database epoch key.
    pub fn unlock(&self, epoch: &ReadKey) -> Result<Zeroizing<Vec<u8>>, Error> {
        self.0.unlock_keyed_object(epoch)
    }
    /// Persist replacement credentials under the supplied epoch key.
    pub fn save(&mut self, epoch: &ReadKey, clear: &[u8]) -> Result<(), Error> {
        self.0.save(epoch, clear)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keyed_local_state_authenticates_reopens_and_isolates_keys() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state");
        let key = LocalStateKey::generate().unwrap();
        let other = LocalStateKey::generate().unwrap();
        let mut store = LocalStateStore::create(&path, &key, b"PUBLIC local state").unwrap();
        assert_eq!(
            &*LocalStateStore::read(&path, &key).unwrap(),
            b"PUBLIC local state"
        );
        assert_eq!(&*store.unlock(&key).unwrap(), b"PUBLIC local state");
        assert!(matches!(store.unlock(&other), Err(Error::Authentication)));
        store.save(&key, b"PUBLIC saved state").unwrap();
        drop(store);
        let reopened = LocalStateStore::open(&path).unwrap();
        assert_eq!(&*reopened.unlock(&key).unwrap(), b"PUBLIC saved state");
        drop(reopened);
        assert!(LocalCredentialStore::open(&path).is_err());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            assert_eq!(
                std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
                0o600
            );
        }
    }

    #[test]
    fn credentials_reject_tampering_and_portable_file_substitution() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credentials");
        let epoch = ReadKey::from_secret(&[31; 32]);
        let store =
            LocalCredentialStore::create(&path, &epoch, b"PUBLIC author seed fixture").unwrap();
        assert_eq!(
            &*store.unlock(&epoch).unwrap(),
            b"PUBLIC author seed fixture"
        );
        drop(store);
        assert!(FileStore::open(&path).is_err());
        let mut bytes = std::fs::read(&path).unwrap();
        let last = bytes.len() - 1;
        bytes[last] ^= 1;
        std::fs::write(&path, &bytes).unwrap();
        let reopened = LocalCredentialStore::open(&path).unwrap();
        assert!(matches!(
            reopened.unlock(&epoch),
            Err(Error::Authentication)
        ));
    }

    #[test]
    fn credential_rotation_reopens_only_with_the_new_epoch() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("credentials");
        let old = ReadKey::from_secret(&[31; 32]);
        let new = ReadKey::from_secret(&[32; 32]);
        let mut store = LocalCredentialStore::create(&path, &old, b"PUBLIC credentials").unwrap();
        store.save(&new, b"PUBLIC next epoch credentials").unwrap();
        drop(store);
        let reopened = LocalCredentialStore::open(&path).unwrap();
        assert!(matches!(reopened.unlock(&old), Err(Error::Authentication)));
        assert_eq!(
            &*reopened.unlock(&new).unwrap(),
            b"PUBLIC next epoch credentials"
        );
    }

    #[test]
    fn changed_local_state_cannot_be_overwritten_by_a_stale_handle() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("state");
        let key = LocalStateKey::generate().unwrap();
        let mut store = LocalStateStore::create(&path, &key, b"PUBLIC local state").unwrap();
        let mut replaced = std::fs::read(&path).unwrap();
        let last = replaced.len() - 1;
        replaced[last] ^= 1;
        std::fs::write(&path, &replaced).unwrap();
        assert!(matches!(
            store.save(&key, b"PUBLIC stale update"),
            Err(Error::Changed)
        ));
        assert_eq!(std::fs::read(&path).unwrap(), replaced);
        assert!(matches!(store.unlock(&key), Err(Error::Changed)));
    }
}
