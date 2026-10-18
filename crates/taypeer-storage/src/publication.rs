//! Durable publication of a complete prepared file.

use crate::{Error, file};
#[cfg(not(windows))]
use std::fs::File;
use std::{io, path::Path};
use tempfile::NamedTempFile;

pub(crate) const DRAFT_DISCARDED: &[u8] = b"TAYPEER-DRAFT-DISCARDED-V1\0";

/// Whether publication may replace an existing destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationMode {
    /// Create a new file without replacing an existing destination.
    Create,
    /// Atomically replace the existing destination, or create it when absent.
    Replace,
}

/// Synchronize and atomically publish a complete prepared file, then confirm
/// publication through the platform's durability barrier. A filename without a directory is relative to the
/// current directory. Prepare the temporary file in the destination directory.
///
/// `AlreadyExists` leaves the destination untouched in `Create` mode. An ordinary
/// I/O error does not confirm publication. `CommitUncertain` means publication
/// may have occurred without confirmed durability: reconcile the destination
/// before retrying or reporting that it was saved.
pub fn publish_file(
    temp: NamedTempFile,
    destination: &Path,
    mode: PublicationMode,
) -> Result<(), Error> {
    publish_with_sync(temp, destination, mode, confirm_published_file)
}

fn publish_with_sync(
    temp: NamedTempFile,
    destination: &Path,
    mode: PublicationMode,
    confirm: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), Error> {
    // Resolve the directory before publication so an invalid destination cannot
    // first replace a file and only then fail path validation.
    let destination = file::canonical_destination(destination)?;
    temp.as_file().sync_all()?;
    #[cfg(windows)]
    publish_windows(temp, &destination, mode)?;
    #[cfg(not(windows))]
    match mode {
        PublicationMode::Create => {
            publish_new(temp, &destination)?;
        }
        PublicationMode::Replace => {
            temp.persist(&destination).map_err(|_| Error::Io)?;
        }
    }
    confirm(&destination).map_err(|_| Error::CommitUncertain)
}

// Windows has no std directory-fsync equivalent. Use a write-through same-volume
// move, then flush the published file through a writable handle. Power-cut
// acceptance remains separate from these OS acknowledgements.
#[cfg(windows)]
fn publish_windows(
    temp: NamedTempFile,
    destination: &Path,
    mode: PublicationMode,
) -> Result<(), Error> {
    if destination.is_dir() {
        return Err(Error::Io);
    }
    let (handle, path) = temp.into_parts();
    drop(handle);
    let result = match mode {
        PublicationMode::Create => atomicwrites::move_atomic(&path, destination),
        PublicationMode::Replace => atomicwrites::replace_atomic(&path, destination),
    };
    result.map_err(|error| {
        if error.kind() == io::ErrorKind::AlreadyExists {
            Error::AlreadyExists
        } else {
            // A failed native move does not prove that the destination is unchanged.
            Error::CommitUncertain
        }
    })
}

pub(crate) fn confirm_published_file(path: &Path) -> io::Result<()> {
    #[cfg(not(windows))]
    {
        File::open(path)?.sync_all()?;
        File::open(file::parent(path).map_err(io::Error::other)?)?.sync_all()
    }
    #[cfg(windows)]
    {
        std::fs::OpenOptions::new()
            .write(true)
            .open(path)?
            .sync_all()
    }
}

pub(crate) fn remove_published_file(path: &Path) -> Result<(), Error> {
    #[cfg(not(windows))]
    {
        remove_with_sync(path, |parent| File::open(parent)?.sync_all())
    }
    #[cfg(windows)]
    {
        use std::io::Write;
        // A logical absence marker can be republished after uncertainty without
        // relying on a directory-fsync operation Windows does not provide.
        let mut marker = NamedTempFile::new_in(file::parent(path)?)?;
        marker.write_all(DRAFT_DISCARDED)?;
        publish_file(marker, path, PublicationMode::Replace)
    }
}

#[cfg(not(windows))]
fn remove_with_sync(path: &Path, sync: impl FnOnce(&Path) -> io::Result<()>) -> Result<(), Error> {
    match std::fs::remove_file(path) {
        Ok(()) => {}
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err(Error::Io),
    }
    // An absent name may be the visible result of an earlier failed fsync.
    sync(file::parent(path)?).map_err(|_| Error::CommitUncertain)
}

pub(crate) fn is_discarded(file: &mut std::fs::File) -> Result<bool, Error> {
    use std::io::{Read, Seek, SeekFrom};
    if file.metadata()?.len() != DRAFT_DISCARDED.len() as u64 {
        return Ok(false);
    }
    let mut bytes = vec![0; DRAFT_DISCARDED.len()];
    file.read_exact(&mut bytes)?;
    file.seek(SeekFrom::Start(0))?;
    Ok(bytes == DRAFT_DISCARDED)
}

#[cfg(not(any(target_os = "android", windows)))]
fn publish_new(temp: NamedTempFile, destination: &Path) -> Result<(), Error> {
    temp.persist_noclobber(destination).map_err(|error| {
        if error.error.kind() == io::ErrorKind::AlreadyExists {
            Error::AlreadyExists
        } else {
            Error::Io
        }
    })?;
    Ok(())
}

#[cfg(target_os = "android")]
fn publish_new(temp: NamedTempFile, destination: &Path) -> Result<(), Error> {
    // Android app SELinux policy prohibits the hard-link fallback used by
    // tempfile. NOREPLACE provides the same atomic no-overwrite contract.
    use rustix::fs::{CWD, RenameFlags, renameat_with};
    renameat_with(CWD, temp.path(), CWD, destination, RenameFlags::NOREPLACE).map_err(|error| {
        if error == rustix::io::Errno::EXIST {
            Error::AlreadyExists
        } else {
            Error::Io
        }
    })?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, io::Write};

    fn staged(directory: &Path, bytes: &[u8]) -> NamedTempFile {
        let mut file = NamedTempFile::new_in(directory).unwrap();
        file.write_all(bytes).unwrap();
        file
    }

    #[test]
    fn create_does_not_replace_existing_content() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("PUBLIC export");
        fs::write(&destination, b"PUBLIC original").unwrap();
        assert_eq!(
            publish_file(
                staged(directory.path(), b"PUBLIC replacement"),
                &destination,
                PublicationMode::Create,
            ),
            Err(Error::AlreadyExists)
        );
        assert_eq!(fs::read(destination).unwrap(), b"PUBLIC original");
    }

    #[test]
    fn create_and_replace_publish_the_complete_prepared_content() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("PUBLIC export");
        for (mode, bytes) in [
            (PublicationMode::Create, b"PUBLIC initial".as_slice()),
            (PublicationMode::Replace, b"PUBLIC replacement".as_slice()),
        ] {
            publish_file(staged(directory.path(), bytes), &destination, mode).unwrap();
            assert_eq!(fs::read(&destination).unwrap(), bytes);
        }
    }

    #[test]
    fn filename_only_destination_uses_current_directory_without_changing_it() {
        // A TempPath owns cleanup even when an assertion fails. The unique name
        // avoids changing the process cwd or colliding with parallel tests.
        let destination = NamedTempFile::new_in(".").unwrap().into_temp_path();
        fs::remove_file(&destination).unwrap();
        let relative = Path::new(destination.file_name().unwrap());
        publish_file(
            staged(Path::new("."), b"PUBLIC relative export"),
            relative,
            PublicationMode::Create,
        )
        .unwrap();
        assert_eq!(fs::read(relative).unwrap(), b"PUBLIC relative export");
    }

    #[test]
    fn destination_failure_leaves_existing_directory_contents_untouched() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("PUBLIC directory");
        fs::create_dir(&destination).unwrap();
        fs::write(destination.join("PUBLIC existing"), b"PUBLIC original").unwrap();
        assert_eq!(
            publish_file(
                staged(directory.path(), b"PUBLIC replacement"),
                &destination,
                PublicationMode::Replace,
            ),
            Err(Error::Io)
        );
        assert_eq!(
            fs::read(destination.join("PUBLIC existing")).unwrap(),
            b"PUBLIC original"
        );
    }

    #[test]
    fn failed_directory_sync_is_uncertain_after_replacement() {
        let directory = tempfile::tempdir().unwrap();
        let destination = directory.path().join("PUBLIC export");
        fs::write(&destination, b"PUBLIC original").unwrap();
        assert_eq!(
            publish_with_sync(
                staged(directory.path(), b"PUBLIC replacement"),
                &destination,
                PublicationMode::Replace,
                |_| Err(io::Error::other("PUBLIC directory sync failure")),
            ),
            Err(Error::CommitUncertain)
        );
        assert_eq!(fs::read(destination).unwrap(), b"PUBLIC replacement");
    }

    #[cfg(not(windows))]
    #[test]
    fn absent_discard_retry_still_requires_a_successful_directory_barrier() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC draft");
        fs::write(&path, b"PUBLIC ciphertext").unwrap();
        let denied = |_: &Path| Err(io::Error::other("PUBLIC barrier failure"));
        assert_eq!(remove_with_sync(&path, denied), Err(Error::CommitUncertain));
        assert!(!path.exists());
        assert_eq!(remove_with_sync(&path, denied), Err(Error::CommitUncertain));
        let observed = std::cell::Cell::new(false);
        remove_with_sync(&path, |_| {
            observed.set(true);
            Ok(())
        })
        .unwrap();
        assert!(observed.get());
    }

    #[test]
    fn draft_tombstone_is_exact_and_never_turns_unknown_bytes_into_absence() {
        let directory = tempfile::tempdir().unwrap();
        let path = directory.path().join("PUBLIC database.taypeer");
        let (store, key) =
            crate::FileStore::create(&path, b"PUBLIC master", b"PUBLIC document").unwrap();
        let draft = file::sibling(&path, ".draft");
        fs::write(&draft, DRAFT_DISCARDED).unwrap();
        assert!(store.load_draft(&key).unwrap().is_none());
        assert!(store.load_binary_draft(&key).unwrap().is_none());

        let snapshot = crate::ArchiveSnapshot::open(
            Path::new(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../tests/fixtures/stable1/populated.taypeer"
            )),
            None,
        )
        .unwrap();
        let copy = taypeer_trust::Digest::of(b"PUBLIC tombstone working copy");
        let signed_draft = file::sibling(&path, &format!(".{copy}.draft"));
        fs::write(&signed_draft, DRAFT_DISCARDED).unwrap();
        assert!(
            crate::read_local_draft(&path, copy, snapshot.chain())
                .unwrap()
                .is_none()
        );
        let mut damaged = DRAFT_DISCARDED.to_vec();
        damaged.push(0);
        fs::write(&draft, &damaged).unwrap();
        fs::write(&signed_draft, &damaged).unwrap();
        assert!(store.load_draft(&key).is_err());
        assert!(store.load_binary_draft(&key).is_err());
        assert!(crate::read_local_draft(&path, copy, snapshot.chain()).is_err());
    }
}
