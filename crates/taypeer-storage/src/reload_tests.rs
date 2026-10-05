//! Reconciliation of an unconfirmed replacement without a second password prompt.

use super::*;

fn create(path: &Path, document: &[u8], blobs: &BlobStore) -> (FileStore, ReadKey) {
    let reader = blobs.bundle(document).unwrap();
    let length = reader.length();
    FileStore::create_stream(path, b"PUBLIC master", reader, length, 500).unwrap()
}

fn replace_unconfirmed(store: &mut FileStore, key: &ReadKey, document: &[u8], blobs: &BlobStore) {
    let reader = blobs.bundle(document).unwrap();
    let length = reader.length();
    let mut temp = NamedTempFile::new_in(parent(&store.path).unwrap()).unwrap();
    crypto::encrypt_stream(&store.header, key, reader, length, &mut temp).unwrap();
    persist(temp, &store.path, false).unwrap();
    // Model the same visible file and retained baseline as a post-rename sync
    // failure without relying on the OS to fail a directory synchronization.
    store.uncertain = true;
}

#[test]
fn unconfirmed_bundle_is_authenticated_and_allows_a_following_commit() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut blobs = BlobStore::new().unwrap();
    let (mut store, key) = create(&path, b"PUBLIC before", &blobs);
    let content = b"PUBLIC retained binary";
    let id = blobs
        .insert(content.as_slice(), content.len() as u64, 100)
        .unwrap();
    replace_unconfirmed(&mut store, &key, b"PUBLIC attempted", &blobs);
    assert_eq!(
        store.save(&key, b"PUBLIC blocked retry"),
        Err(Error::CommitUncertain)
    );

    let (document, restored) = store.reload_bundle(&key).unwrap();
    assert_eq!(document.as_slice(), b"PUBLIC attempted");
    let mut bytes = Vec::new();
    restored
        .reader(&id)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, content);
    let reader = restored.bundle(b"PUBLIC following commit").unwrap();
    let length = reader.length();
    store.save_stream(&key, reader, length).unwrap();
    drop(store);

    let mut reopened = FileStore::open(&path).unwrap();
    let (_, document, restored) = reopened.unlock_bundle(b"PUBLIC master").unwrap();
    assert_eq!(document.as_slice(), b"PUBLIC following commit");
    assert_eq!(restored.length(&id), Some(content.len() as u64));
}

#[test]
fn different_password_wrapper_does_not_advance_or_clear_the_baseline() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let blobs = BlobStore::new().unwrap();
    let (mut store, key) = create(&path, b"PUBLIC before", &blobs);
    let original_header = store.header.clone();
    let original_fingerprint = store.fingerprint;
    let other_path = directory.path().join("PUBLIC other.taypeer");
    let (_other, _) = create(&other_path, b"PUBLIC unrelated", &blobs);
    fs::copy(other_path, path).unwrap();
    store.uncertain = true;

    assert!(matches!(store.reload_bundle(&key), Err(Error::Changed)));
    assert_eq!(store.header, original_header);
    assert_eq!(store.fingerprint, original_fingerprint);
    assert_eq!(
        store.save(&key, b"PUBLIC retry"),
        Err(Error::CommitUncertain)
    );
}

#[test]
fn corrupt_unconfirmed_bundle_cannot_clear_uncertainty_or_return_candidates() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let blobs = BlobStore::new().unwrap();
    let (mut store, key) = create(&path, b"PUBLIC before", &blobs);
    let original_header = store.header.clone();
    let original_fingerprint = store.fingerprint;
    replace_unconfirmed(&mut store, &key, b"PUBLIC attempted", &blobs);
    let mut corrupted = fs::read(&path).unwrap();
    *corrupted.last_mut().unwrap() ^= 1;
    fs::write(path, corrupted).unwrap();

    assert!(matches!(
        store.reload_bundle(&key),
        Err(Error::Authentication)
    ));
    assert_eq!(store.header, original_header);
    assert_eq!(store.fingerprint, original_fingerprint);
    assert_eq!(
        store.save(&key, b"PUBLIC retry"),
        Err(Error::CommitUncertain)
    );
}
