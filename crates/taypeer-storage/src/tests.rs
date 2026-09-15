use super::*;
use std::fs;

#[test]
fn streaming_authenticates_order_completion_and_snapshot_identity() {
    use crate::crypto;
    let (header, key) = crypto::create_header(b"PUBLIC stream", 500).unwrap();
    let clear = vec![0x52; 2 * 1024 * 1024 + 3];
    let mut first = Vec::new();
    let first_header = crypto::encrypt_stream(
        &header,
        &key,
        clear.as_slice(),
        clear.len() as u64,
        &mut first,
    )
    .unwrap();
    let mut second = Vec::new();
    crypto::encrypt_stream(
        &header,
        &key,
        clear.as_slice(),
        clear.len() as u64,
        &mut second,
    )
    .unwrap();
    let mut decoded = Vec::new();
    crypto::decrypt_stream(
        &first_header,
        &key,
        &mut &first[crypto::HEADER..],
        &mut decoded,
    )
    .unwrap();
    assert_eq!(decoded, clear);
    let frame = 1024 * 1024 + 40;
    let mut changed = first.clone();
    changed[crypto::HEADER..crypto::HEADER + frame]
        .copy_from_slice(&second[crypto::HEADER..crypto::HEADER + frame]);
    assert!(
        crypto::decrypt_stream(
            &first_header,
            &key,
            &mut &changed[crypto::HEADER..],
            &mut std::io::sink()
        )
        .is_err()
    );
    let mut swapped = first.clone();
    swapped[crypto::HEADER..crypto::HEADER + frame]
        .copy_from_slice(&first[crypto::HEADER + frame..crypto::HEADER + 2 * frame]);
    assert!(
        crypto::decrypt_stream(
            &first_header,
            &key,
            &mut &swapped[crypto::HEADER..],
            &mut std::io::sink()
        )
        .is_err()
    );
    assert!(
        crypto::decrypt_stream(
            &first_header,
            &key,
            &mut &first[crypto::HEADER..first.len() - 40],
            &mut std::io::sink()
        )
        .is_err()
    );
    first.push(0);
    assert!(
        crypto::decrypt_stream(
            &first_header,
            &key,
            &mut &first[crypto::HEADER..],
            &mut std::io::sink()
        )
        .is_err()
    );
}

#[test]
fn stream_input_length_failure_preserves_the_working_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("PUBLIC.taypeer");
    let (mut store, key) = FileStore::create(&path, b"PUBLIC master", b"PUBLIC initial").unwrap();
    let before = fs::read(&path).unwrap();
    assert!(
        store
            .save_stream(&key, b"PUBLIC short".as_slice(), 4096)
            .is_err()
    );
    assert_eq!(before, fs::read(&path).unwrap());
    assert!(
        store
            .save_stream(&key, b"PUBLIC trailing".as_slice(), 1)
            .is_err()
    );
    assert_eq!(before, fs::read(&path).unwrap());
}

#[test]
fn authentication_versions_exclusivity_and_repeated_atomic_saves() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.taypeer");
    let (mut file, key) = FileStore::create(&path, b"PUBLIC password", b"PUBLIC original").unwrap();
    assert!(matches!(FileStore::open(&path), Err(Error::Busy)));
    for index in 0..12 {
        file.save(&key, format!("PUBLIC {index}").as_bytes())
            .unwrap();
    }
    assert!(!directory.path().join("fixture.taypeer.backups").exists());
    let (_, clear) = file.unlock(b"PUBLIC password").unwrap();
    assert_eq!(clear.as_slice(), b"PUBLIC 11");
    let original = fs::read(&path).unwrap();
    let mut changed = original.clone();
    *changed.last_mut().unwrap() ^= 1;
    fs::write(&path, &changed).unwrap();
    assert_eq!(file.save(&key, b"PUBLIC rejected"), Err(Error::Changed));
    assert!(matches!(
        file.unlock(b"PUBLIC password"),
        Err(Error::Authentication)
    ));
    assert_eq!(fs::read(&path).unwrap(), changed);
    drop(file);
    changed[8] = 0;
    fs::write(&path, &changed).unwrap();
    assert!(matches!(
        FileStore::open(&path),
        Err(Error::UnsupportedVersion)
    ));
    assert_eq!(fs::read(&path).unwrap(), changed);
    for schema in [1, 2] {
        changed[8..12].copy_from_slice(&[1, 0, schema, 0]);
        fs::write(&path, &changed).unwrap();
        assert!(matches!(
            FileStore::open(&path),
            Err(Error::UnsupportedVersion)
        ));
        assert_eq!(fs::read(&path).unwrap(), changed);
    }
    fs::write(&path, original).unwrap();
    assert!(matches!(
        FileStore::create(&path, b"PUBLIC password", b"PUBLIC replacement"),
        Err(Error::AlreadyExists)
    ));
}

#[test]
fn empty_password_and_truncated_input_never_create_or_replace_a_database() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.taypeer");
    assert!(matches!(
        FileStore::create(&path, b"", b"PUBLIC"),
        Err(Error::EmptyPassword)
    ));
    assert!(!path.exists());
    for bytes in [b"".as_slice(), b"TAYPEER\0", b"PUBLIC invalid"] {
        fs::write(&path, bytes).unwrap();
        assert!(matches!(FileStore::open(&path), Err(Error::InvalidFile)));
        assert_eq!(fs::read(&path).unwrap(), bytes);
    }
}

#[test]
fn automatic_saves_ignore_existing_backup_files_and_directories() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("PUBLIC.taypeer");
    let (mut file, key) = FileStore::create(&path, b"PUBLIC password", b"PUBLIC original").unwrap();
    let backups = directory.path().join("PUBLIC.taypeer.backups");
    fs::create_dir(&backups).unwrap();
    for index in 0..12 {
        fs::write(
            backups.join(format!("{index:020}.taypeer")),
            b"PUBLIC old copy",
        )
        .unwrap();
    }
    file.save(&key, b"PUBLIC saved").unwrap();
    assert_eq!(fs::read_dir(&backups).unwrap().count(), 12);
    for entry in fs::read_dir(&backups).unwrap() {
        assert_eq!(fs::read(entry.unwrap().path()).unwrap(), b"PUBLIC old copy");
    }
    fs::rename(&backups, directory.path().join("PUBLIC untouched copies")).unwrap();
    fs::write(&backups, b"PUBLIC unrelated file").unwrap();
    file.save(&key, b"PUBLIC saved again").unwrap();
    assert_eq!(fs::read(&backups).unwrap(), b"PUBLIC unrelated file");
}
