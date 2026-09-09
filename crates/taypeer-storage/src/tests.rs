use super::*;
use std::fs;

#[test]
fn authentication_versions_exclusivity_and_backup_retention() {
    let directory = tempfile::tempdir().unwrap();
    let path = directory.path().join("fixture.taypeer");
    let (mut file, key) = FileStore::create(&path, b"PUBLIC password", b"PUBLIC original").unwrap();
    assert!(matches!(FileStore::open(&path), Err(Error::Busy)));
    for index in 0..12 {
        file.save(&key, format!("PUBLIC {index}").as_bytes())
            .unwrap();
    }
    let backups: Vec<_> = fs::read_dir(directory.path().join("fixture.taypeer.backups"))
        .unwrap()
        .map(|entry| entry.unwrap().path())
        .collect();
    assert_eq!(backups.len(), 10);
    let mut backup = FileStore::open(&backups[0]).unwrap();
    let (_, clear) = backup.unlock(b"PUBLIC password").unwrap();
    assert!(clear.starts_with(b"PUBLIC "));
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
    changed[8] = 1;
    fs::write(&path, &changed).unwrap();
    assert!(matches!(
        FileStore::open(&path),
        Err(Error::UnsupportedVersion)
    ));
    assert_eq!(fs::read(&path).unwrap(), changed);
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
