//! Synthetic bounded streams and malformed catalogs, without persistent fixture binaries.
use super::*;
use std::{
    collections::BTreeSet,
    io::{Cursor, Read},
};

#[test]
fn bundle_deduplicates_aliases_roundtrips_and_rejects_corrupt_sections() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let mut blobs = BlobStore::new().unwrap();
    let content = b"PUBLIC ORIGINAL BINARY CONTENT";
    let a = blobs
        .insert(content.as_slice(), content.len() as u64, 100)
        .unwrap();
    let b = blobs
        .insert(content.as_slice(), content.len() as u64, 100)
        .unwrap();
    assert_eq!(
        blobs.unique_bytes(&BTreeSet::from([a.clone(), b.clone()])),
        content.len() as u64
    );
    let reader = blobs.bundle(b"PUBLIC document").unwrap();
    let length = reader.length();
    let (mut file, key) =
        FileStore::create_stream(&path, b"PUBLIC master", reader, length, 500).unwrap();
    let (_, document, restored) = file.unlock_bundle(b"PUBLIC master").unwrap();
    assert_eq!(document.as_slice(), b"PUBLIC document");
    let mut bytes = Vec::new();
    restored
        .reader(&b)
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    assert_eq!(bytes, content);
    file.save_binary_draft(&key, b"PUBLIC draft", &blobs)
        .unwrap();
    let crate::BinaryDraft {
        document: draft,
        blobs: staged,
    } = file.load_binary_draft(&key).unwrap().unwrap();
    assert_eq!(draft.as_slice(), b"PUBLIC draft");
    assert_eq!(staged.length(&a), Some(content.len() as u64));
    let encoded = std::fs::read(&path).unwrap();
    assert!(!encoded.windows(content.len()).any(|w| w == content));
    let mut corrupt = encoded.clone();
    *corrupt.last_mut().unwrap() ^= 1;
    std::fs::write(&path, corrupt).unwrap();
    assert!(matches!(
        file.unlock_bundle(b"PUBLIC master"),
        Err(Error::Authentication)
    ));
    std::fs::write(&path, &encoded).unwrap();
    let draft_path = path.with_file_name("PUBLIC.taypeer.draft");
    std::fs::write(draft_path, encoded).unwrap();
    assert!(matches!(
        file.load_binary_draft(&key),
        Err(Error::Authentication)
    ));
}

#[test]
fn catalog_rejects_duplicate_ids_unknown_fields_and_false_lengths() {
    let mut blobs = BlobStore::new().unwrap();
    blobs.insert(b"PUBLIC".as_slice(), 6, 6).unwrap();
    let mut clear = Vec::new();
    blobs
        .bundle(b"PUBLIC doc")
        .unwrap()
        .read_to_end(&mut clear)
        .unwrap();
    let length = u64::from_le_bytes(clear[8..16].try_into().unwrap()) as usize;
    let original: serde_json::Value = serde_json::from_slice(&clear[16..16 + length]).unwrap();
    for kind in [0, 1, 2] {
        let mut catalog = original.clone();
        match kind {
            0 => {
                let id = catalog["sections"][0]["ids"][0].clone();
                catalog["sections"][0]["ids"]
                    .as_array_mut()
                    .unwrap()
                    .push(id);
            }
            1 => catalog["unknown"] = true.into(),
            _ => catalog["sections"][0]["length"] = u64::MAX.into(),
        }
        let metadata = serde_json::to_vec(&catalog).unwrap();
        let mut candidate = b"TAYBLOB3".to_vec();
        candidate.extend((metadata.len() as u64).to_le_bytes());
        candidate.extend(metadata);
        candidate.extend(&clear[16 + length..]);
        assert!(BlobStore::read_bundle(Cursor::new(candidate)).is_err());
    }
}

struct Pattern {
    remaining: u64,
}
impl Read for Pattern {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        assert!(bytes.len() <= 1024 * 1024);
        let count = (bytes.len() as u64).min(self.remaining) as usize;
        bytes[..count].fill(42);
        self.remaining -= count as u64;
        Ok(count)
    }
}

#[test]
fn binary_stream_exceeds_document_limit_without_a_file_sized_buffer() {
    let length = MAX_FILE_SIZE as u64 + 1;
    let mut blobs = BlobStore::new().unwrap();
    let id = blobs
        .insert(Pattern { remaining: length }, length, length)
        .unwrap();
    let mut reader = blobs.bundle(b"PUBLIC small document").unwrap();
    let expected = reader.length();
    assert_eq!(
        std::io::copy(&mut reader, &mut std::io::sink()).unwrap(),
        expected
    );
    drop(reader);
    assert_eq!(blobs.length(&id), Some(length));
    assert!(matches!(
        blobs.insert(b"PUBLIC".as_slice(), 6, 5),
        Err(Error::TooLarge)
    ));
}
