//! The public ciphertext writer rejects invalid consumers without panic or format changes.
use super::*;
use std::io::{self, Write};

#[test]
fn verified_ciphertext_copy_keeps_the_exact_archive_and_accepts_partial_writes() {
    struct Partial(Vec<u8>);
    impl Write for Partial {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            let count = bytes.len().min(17);
            self.0.extend_from_slice(&bytes[..count]);
            Ok(count)
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC verified ciphertext.taypeer");
    let fixture = Fixture::new();
    let (candidate, metadata) = fixture.initial();
    let store = ArchiveStore::create(&path, &candidate, metadata, None).unwrap();
    let snapshot = store.snapshot();
    let original = fs::read(&path).unwrap();
    let mut output = Partial(Vec::new());
    snapshot.copy_verified_ciphertext(&mut output).unwrap();
    assert_eq!(output.0, original);
    assert_eq!(Digest::of(&output.0), snapshot.fingerprint());
    assert_eq!(fs::read(path).unwrap(), original);
}

#[test]
fn zero_overlong_and_throwing_ciphertext_consumers_return_io_without_panicking() {
    enum Invalid {
        Zero,
        Overlong,
        Throwing,
    }
    impl Write for Invalid {
        fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
            match self {
                Self::Zero => Ok(0),
                Self::Overlong => Ok(bytes.len() + 1),
                Self::Throwing => Err(io::ErrorKind::PermissionDenied.into()),
            }
        }
        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC failed copy.taypeer");
    let fixture = Fixture::new();
    let (candidate, metadata) = fixture.initial();
    let store = ArchiveStore::create(&path, &candidate, metadata, None).unwrap();
    let original = fs::read(&path).unwrap();
    for mut consumer in [Invalid::Zero, Invalid::Overlong, Invalid::Throwing] {
        assert!(matches!(
            store.snapshot().copy_verified_ciphertext(&mut consumer),
            Err(Error::Io)
        ));
        assert_eq!(fs::read(&path).unwrap(), original);
    }
    let mut retry = Vec::new();
    store
        .snapshot()
        .copy_verified_ciphertext(&mut retry)
        .unwrap();
    assert_eq!(retry, original);
}
