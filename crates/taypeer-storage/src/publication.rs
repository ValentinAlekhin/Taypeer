//! Durable publication of a complete prepared file.

use crate::{Error, file};
use std::{fs::File, io, path::Path};
use tempfile::NamedTempFile;

/// Whether publication may replace an existing destination.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum PublicationMode {
    /// Create a new file without replacing an existing destination.
    Create,
    /// Atomically replace the existing destination, or create it when absent.
    Replace,
}

/// Synchronize and atomically publish a complete prepared file, then synchronize
/// its destination directory. A filename without a directory is relative to the
/// current directory. Prepare the temporary file in the destination directory.
///
/// `AlreadyExists` leaves the destination untouched in `Create` mode. An ordinary
/// I/O error does not confirm publication. `CommitUncertain` means publication
/// occurred but directory synchronization failed: reconcile the destination
/// before retrying or reporting that it was saved.
pub fn publish_file(
    temp: NamedTempFile,
    destination: &Path,
    mode: PublicationMode,
) -> Result<(), Error> {
    publish_with_sync(temp, destination, mode, |directory| {
        File::open(directory)?.sync_all()
    })
}

fn publish_with_sync(
    temp: NamedTempFile,
    destination: &Path,
    mode: PublicationMode,
    sync_directory: impl FnOnce(&Path) -> io::Result<()>,
) -> Result<(), Error> {
    // Resolve the directory before publication so an invalid destination cannot
    // first replace a file and only then fail path validation.
    let destination = file::canonical_destination(destination)?;
    let directory = file::parent(&destination)?;
    temp.as_file().sync_all()?;
    match mode {
        PublicationMode::Create => {
            publish_new(temp, &destination)?;
        }
        PublicationMode::Replace => {
            temp.persist(&destination).map_err(|_| Error::Io)?;
        }
    }
    sync_directory(directory).map_err(|_| Error::CommitUncertain)
}

#[cfg(not(target_os = "android"))]
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
}
