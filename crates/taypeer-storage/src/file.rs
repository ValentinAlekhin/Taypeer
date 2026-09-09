use crate::{Error, MAX_FILE_SIZE, ReadKey, crypto};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

/// One exclusively opened local file. The handle holds no plaintext or read key.
/// Sidecars remain local; copying the database does not require them for reading.
pub struct FileStore {
    path: PathBuf,
    _lock: File,
    bytes: Vec<u8>,
    uncertain: bool,
}

fn sibling(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(suffix);
    PathBuf::from(name)
}
fn parent(path: &Path) -> Result<&Path, Error> {
    path.parent()
        .filter(|p| !p.as_os_str().is_empty())
        .ok_or(Error::Io)
}
fn canonical_destination(path: &Path) -> Result<PathBuf, Error> {
    let name = path.file_name().ok_or(Error::Io)?;
    let directory = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    Ok(directory.canonicalize()?.join(name))
}
fn read(path: &Path) -> Result<Vec<u8>, Error> {
    let file = File::open(path)?;
    if file.metadata()?.len() > MAX_FILE_SIZE as u64 {
        return Err(Error::TooLarge);
    }
    let mut bytes = Vec::new();
    file.take(MAX_FILE_SIZE as u64 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > MAX_FILE_SIZE {
        return Err(Error::TooLarge);
    }
    Ok(bytes)
}
fn atomic_write(path: &Path, bytes: &[u8], create: bool) -> Result<(), Error> {
    let directory = parent(path)?;
    let mut temp = NamedTempFile::new_in(directory)?;
    temp.write_all(bytes)?;
    temp.as_file().sync_all()?;
    if create {
        temp.persist_noclobber(path).map_err(|error| {
            if error.error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::AlreadyExists
            } else {
                Error::Io
            }
        })?;
    } else {
        temp.persist(path).map_err(|_| Error::Io)?;
    }
    File::open(directory)
        .and_then(|directory| directory.sync_all())
        .map_err(|_| Error::CommitUncertain)
}
fn lock(path: &Path) -> Result<File, Error> {
    let mut options = OpenOptions::new();
    options.read(true).write(true).create(true).truncate(false);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    let file = options.open(sibling(path, ".lock"))?;
    file.try_lock().map_err(|error| match error {
        std::fs::TryLockError::WouldBlock => Error::Busy,
        std::fs::TryLockError::Error(_) => Error::Io,
    })?;
    Ok(file)
}

impl FileStore {
    /// Create a new encrypted database, refusing to replace an existing path.
    pub fn create(path: &Path, password: &[u8], clear: &[u8]) -> Result<(Self, ReadKey), Error> {
        let path = canonical_destination(path)?;
        let guard = lock(&path)?;
        if path.try_exists()? {
            return Err(Error::AlreadyExists);
        }
        let (bytes, key) = crypto::create(password, clear)?;
        atomic_write(&path, &bytes, true)?;
        Ok((
            Self {
                path,
                _lock: guard,
                bytes,
                uncertain: false,
            },
            key,
        ))
    }
    /// Open and structurally check the encrypted file without deriving a key.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let path = path.canonicalize()?;
        let guard = lock(&path)?;
        let bytes = read(&path)?;
        crypto::validate(&bytes)?;
        Ok(Self {
            path,
            _lock: guard,
            bytes,
            uncertain: false,
        })
    }
    /// Selected canonical path, suitable for local presentation only.
    pub fn path(&self) -> &Path {
        &self.path
    }
    /// Authenticate the file and return an owned key and zeroizing plaintext buffer.
    pub fn unlock(&mut self, password: &[u8]) -> Result<(ReadKey, Zeroizing<Vec<u8>>), Error> {
        let bytes = read(&self.path)?;
        let result = crypto::unlock(&bytes, password)?;
        self.bytes = bytes;
        self.uncertain = false;
        Ok(result)
    }
    /// Save ciphertext before callers publish the corresponding in-memory candidate.
    /// Any error leaves the caller responsible for retaining its unconfirmed form.
    pub fn save(&mut self, key: &ReadKey, clear: &[u8]) -> Result<(), Error> {
        if self.uncertain {
            return Err(Error::CommitUncertain);
        }
        if read(&self.path)? != self.bytes {
            return Err(Error::Changed);
        }
        let next = crypto::encrypt(&self.bytes, key, clear)?;
        let directory = sibling(&self.path, ".backups");
        fs::create_dir_all(&directory)?;
        let mut backups = backup_files(&directory)?;
        // A failed attempt may already have saved this exact source. Retrying must not
        // evict distinct historical snapshots by accumulating duplicate backups.
        let already_backed_up = match backups.last() {
            Some((_, path)) => read(path)? == self.bytes,
            None => false,
        };
        if !already_backed_up {
            let generation = backups
                .last()
                .map_or(Some(0), |(n, _)| n.checked_add(1))
                .ok_or(Error::Io)?;
            let path = directory.join(format!("{generation:020}.taypeer"));
            atomic_write(&path, &self.bytes, true)?;
            backups.push((generation, path));
        }
        // Directory creation itself must also reach disk before replacing the source.
        File::open(parent(&self.path)?)?.sync_all()?;
        let result = atomic_write(&self.path, &next, false);
        if result == Err(Error::CommitUncertain) {
            self.uncertain = true;
        }
        result?;
        self.bytes = next;
        for (_, path) in backups.iter().take(backups.len().saturating_sub(10)) {
            // Retention cleanup failure is reported as uncertain completion, never a false success.
            if fs::remove_file(path).is_err() {
                self.uncertain = true;
                return Err(Error::CommitUncertain);
            }
        }
        File::open(&directory)
            .and_then(|directory| directory.sync_all())
            .map_err(|_| {
                self.uncertain = true;
                Error::CommitUncertain
            })?;
        Ok(())
    }
    /// Save a separately encrypted local interrupted form; it is not a confirmed revision.
    pub fn save_draft(&self, key: &ReadKey, clear: &[u8]) -> Result<(), Error> {
        let bytes = crypto::seal_draft(&self.bytes, key, clear)?;
        atomic_write(&sibling(&self.path, ".draft"), &bytes, false)
    }
    /// Read a local interrupted form, distinguishing absence from corruption.
    pub fn load_draft(&self, key: &ReadKey) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        let path = sibling(&self.path, ".draft");
        if !path.try_exists()? {
            return Ok(None);
        }
        crypto::open_draft(&self.bytes, key, &read(&path)?).map(Some)
    }
    /// Explicitly discard the local interrupted form, syncing its removal.
    pub fn discard_draft(&self) -> Result<(), Error> {
        match fs::remove_file(sibling(&self.path, ".draft")) {
            Ok(()) => File::open(parent(&self.path)?)?
                .sync_all()
                .map_err(Error::from),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(_) => Err(Error::Io),
        }
    }
}
fn backup_files(directory: &Path) -> Result<Vec<(u64, PathBuf)>, Error> {
    let mut files = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let path = entry.path();
        if path.extension().is_some_and(|ext| ext == "taypeer") {
            let generation = path
                .file_stem()
                .and_then(|s| s.to_str())
                .and_then(|s| s.parse().ok())
                .ok_or(Error::Io)?;
            files.push((generation, path));
        }
    }
    files.sort_by_key(|(n, _)| *n);
    Ok(files)
}
