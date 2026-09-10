use crate::{BlobStore, Error, MAX_FILE_SIZE, ReadKey, crypto, stream::DecryptReader};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};
use tempfile::NamedTempFile;
use zeroize::Zeroizing;

/// Authenticated local draft candidates, unpublished until the service validates references.
pub struct BinaryDraft {
    /// Serialized editor state in a zeroizing buffer.
    pub document: Zeroizing<Vec<u8>>,
    /// Independently staged immutable content owned by the draft.
    pub blobs: BlobStore,
}

/// One exclusively opened local file. The handle holds no plaintext or read key.
/// Sidecars remain local; copying the database does not require them for reading.
pub struct FileStore {
    path: PathBuf,
    _lock: File,
    header: Vec<u8>,
    fingerprint: [u8; 32],
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

fn fingerprint(file: &mut File) -> Result<[u8; 32], Error> {
    file.seek(SeekFrom::Start(0))?;
    let length = file.metadata()?.len();
    if length > crypto::MAX_ENCODED_SIZE {
        return Err(Error::TooLarge);
    }
    let mut bounded = (&mut *file).take(length + 1);
    let mut consumed = 0;
    let mut hash = Sha256::new();
    let mut buffer = vec![0; 1024 * 1024];
    loop {
        let n = bounded.read(&mut buffer)?;
        consumed += n as u64;
        if consumed > length {
            return Err(Error::Changed);
        }
        if n == 0 {
            break;
        }
        hash.update(&buffer[..n]);
    }
    if consumed != length {
        return Err(Error::Changed);
    }
    file.seek(SeekFrom::Start(0))?;
    Ok(hash.finalize().into())
}

fn header(file: &mut File) -> Result<Vec<u8>, Error> {
    let mut bytes = vec![0; crypto::HEADER];
    file.read_exact(&mut bytes)
        .map_err(|_| Error::InvalidFile)?;
    crypto::validate(&bytes, file.metadata()?.len())?;
    Ok(bytes)
}

fn persist(temp: NamedTempFile, path: &Path, create: bool) -> Result<(), Error> {
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
    File::open(parent(path)?)?
        .sync_all()
        .map_err(|_| Error::CommitUncertain)
}

fn copy_durable(source: &Path, destination: &Path) -> Result<(), Error> {
    let mut input = File::open(source)?;
    let mut output = NamedTempFile::new_in(parent(destination)?)?;
    let length = input.metadata()?.len();
    if length > crypto::MAX_ENCODED_SIZE {
        return Err(Error::TooLarge);
    }
    if std::io::copy(&mut (&mut input).take(length + 1), &mut output)? != length {
        return Err(Error::Changed);
    }
    persist(output, destination, true)
}

impl FileStore {
    /// Create a new encrypted database without replacing an existing path.
    pub fn create(path: &Path, password: &[u8], clear: &[u8]) -> Result<(Self, ReadKey), Error> {
        if clear.len() > MAX_FILE_SIZE {
            return Err(Error::TooLarge);
        }
        Self::create_stream(path, password, clear, clear.len() as u64, 1000)
    }

    /// Create a bounded, authenticated stream without buffering the entire payload.
    /// Input must contain exactly `length` bytes; failure never publishes a partial file.
    pub fn create_stream(
        path: &Path,
        password: &[u8],
        clear: impl Read,
        length: u64,
        target_ms: u32,
    ) -> Result<(Self, ReadKey), Error> {
        let path = canonical_destination(path)?;
        let guard = lock(&path)?;
        if path.try_exists()? {
            return Err(Error::AlreadyExists);
        }
        let (initial, key) = crypto::create_header(password, target_ms)?;
        let mut temp = NamedTempFile::new_in(parent(&path)?)?;
        let header = crypto::encrypt_stream(&initial, &key, clear, length, &mut temp)?;
        let fingerprint = fingerprint(temp.as_file_mut())?;
        persist(temp, &path, true)?;
        Ok((
            Self {
                path,
                _lock: guard,
                header,
                fingerprint,
                uncertain: false,
            },
            key,
        ))
    }

    /// Exclusively open and structurally validate the file without a read key.
    pub fn open(path: &Path) -> Result<Self, Error> {
        let path = path.canonicalize()?;
        let guard = lock(&path)?;
        let mut file = File::open(&path)?;
        let header = header(&mut file)?;
        let fingerprint = fingerprint(&mut file)?;
        Ok(Self {
            path,
            _lock: guard,
            header,
            fingerprint,
            uncertain: false,
        })
    }

    /// Canonical path of this local working copy; never part of the portable document.
    pub fn path(&self) -> &Path {
        &self.path
    }

    /// Open a sectioned payload, authenticating all sections before returning candidates.
    pub fn unlock_bundle(
        &mut self,
        password: &[u8],
    ) -> Result<(ReadKey, Zeroizing<Vec<u8>>, BlobStore), Error> {
        let mut input = File::open(&self.path)?;
        let candidate = header(&mut input)?;
        let before = fingerprint(&mut input)?;
        input.seek(SeekFrom::Start(crypto::HEADER as u64))?;
        let key = crypto::unlock_key(&candidate, password)?;
        let (document, blobs) =
            BlobStore::read_bundle(DecryptReader::new(&mut input, &key, candidate.clone())?)?;
        if fingerprint(&mut input)? != before {
            return Err(Error::Changed);
        }
        self.header = candidate;
        self.fingerprint = before;
        self.uncertain = false;
        Ok((key, document, blobs))
    }

    /// Atomically persist an independently encrypted binary draft bundle.
    pub fn save_binary_draft(
        &self,
        key: &ReadKey,
        document: &[u8],
        blobs: &BlobStore,
    ) -> Result<(), Error> {
        let reader = blobs.bundle(document)?;
        let length = reader.length();
        let mut header = self.header.clone();
        header[..8].copy_from_slice(b"TAYDRFT3");
        let mut temp = NamedTempFile::new_in(parent(&self.path)?)?;
        crypto::encrypt_stream(&header, key, reader, length, &mut temp)?;
        persist(temp, &sibling(&self.path, ".draft"), false)
    }

    /// Read the entire local draft bundle; missing and invalid drafts are distinct.
    pub fn load_binary_draft(&self, key: &ReadKey) -> Result<Option<BinaryDraft>, Error> {
        let path = sibling(&self.path, ".draft");
        if !path.try_exists()? {
            return Ok(None);
        }
        let mut input = File::open(path)?;
        let mut header = vec![0; crypto::HEADER];
        input.read_exact(&mut header)?;
        if &header[..8] != b"TAYDRFT3" || header[8..104] != self.header[8..104] {
            return Err(Error::Authentication);
        }
        if input.metadata()?.len() > crypto::MAX_ENCODED_SIZE {
            return Err(Error::TooLarge);
        }
        let (document, blobs) = BlobStore::read_bundle(DecryptReader::new(input, key, header)?)?;
        Ok(Some(BinaryDraft { document, blobs }))
    }

    /// Authenticate a bounded document into a zeroizing buffer.
    /// Larger payloads must use the streaming API and an atomic destination.
    pub fn unlock(&mut self, password: &[u8]) -> Result<(ReadKey, Zeroizing<Vec<u8>>), Error> {
        let mut input = File::open(&self.path)?;
        let candidate = header(&mut input)?;
        if crypto::payload_length(&candidate)? > MAX_FILE_SIZE as u64 {
            return Err(Error::TooLarge);
        }
        let mut clear = Zeroizing::new(Vec::new());
        let key = self.unlock_reader(password, &mut input, candidate, &mut *clear)?;
        Ok((key, clear))
    }

    /// Authenticate the stream into a provisional sink, returning its read key only on success.
    /// A failing stream may have written a prefix: the caller MUST discard the sink
    /// on error and must not publish/use any output until this method returns Ok.
    pub fn unlock_to(
        &mut self,
        password: &[u8],
        output: &mut impl Write,
    ) -> Result<ReadKey, Error> {
        let mut input = File::open(&self.path)?;
        let candidate = header(&mut input)?;
        self.unlock_reader(password, &mut input, candidate, output)
    }

    fn unlock_reader(
        &mut self,
        password: &[u8],
        input: &mut File,
        header: Vec<u8>,
        output: &mut impl Write,
    ) -> Result<ReadKey, Error> {
        let fingerprint_before = fingerprint(input)?;
        input.seek(SeekFrom::Start(crypto::HEADER as u64))?;
        let key = crypto::unlock_key(&header, password)?;
        crypto::decrypt_stream(&header, &key, input, output)?;
        if fingerprint(input)? != fingerprint_before {
            return Err(Error::Changed);
        }
        self.header = header;
        self.fingerprint = fingerprint_before;
        self.uncertain = false;
        Ok(key)
    }

    /// Encrypt and commit a document before callers publish their in-memory candidate.
    pub fn save(&mut self, key: &ReadKey, clear: &[u8]) -> Result<(), Error> {
        if clear.len() > MAX_FILE_SIZE {
            return Err(Error::TooLarge);
        }
        self.save_stream(key, clear, clear.len() as u64)
    }

    /// Commit a stream with bounded memory and exactly ten previous automatic snapshots.
    pub fn save_stream(
        &mut self,
        key: &ReadKey,
        clear: impl Read,
        length: u64,
    ) -> Result<(), Error> {
        if self.uncertain {
            return Err(Error::CommitUncertain);
        }
        self.check_unchanged()?;
        let mut temp = NamedTempFile::new_in(parent(&self.path)?)?;
        let next = crypto::encrypt_stream(&self.header, key, clear, length, &mut temp)?;
        let next_fingerprint = fingerprint(temp.as_file_mut())?;
        let directory = sibling(&self.path, ".backups");
        fs::create_dir_all(&directory)?;
        let mut backups = backup_files(&directory)?;
        let already_backed_up = match backups.last() {
            Some((_, path)) => fingerprint(&mut File::open(path)?)? == self.fingerprint,
            None => false,
        };
        if !already_backed_up {
            let generation = backups
                .last()
                .map_or(Some(0), |(n, _)| n.checked_add(1))
                .ok_or(Error::Io)?;
            let path = directory.join(format!("{generation:020}.taypeer"));
            copy_durable(&self.path, &path)?;
            if fingerprint(&mut File::open(&path)?)? != self.fingerprint {
                return Err(Error::Changed);
            }
            backups.push((generation, path));
        }
        File::open(parent(&self.path)?)?.sync_all()?;
        self.check_unchanged()?;
        let result = persist(temp, &self.path, false);
        if result == Err(Error::CommitUncertain) {
            self.uncertain = true;
        }
        result?;
        self.header = next;
        self.fingerprint = next_fingerprint;
        for (_, path) in backups.iter().take(backups.len().saturating_sub(10)) {
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

    fn check_unchanged(&self) -> Result<(), Error> {
        if fingerprint(&mut File::open(&self.path)?)? != self.fingerprint {
            return Err(Error::Changed);
        }
        Ok(())
    }

    /// Save a separately encrypted local form, never a confirmed revision.
    pub fn save_draft(&self, key: &ReadKey, clear: &[u8]) -> Result<(), Error> {
        let bytes = crypto::seal_draft(&self.header, key, clear)?;
        atomic_write(&sibling(&self.path, ".draft"), &bytes, false)
    }

    /// Read the local form, distinguishing absence from corrupt ciphertext.
    pub fn load_draft(&self, key: &ReadKey) -> Result<Option<Zeroizing<Vec<u8>>>, Error> {
        let path = sibling(&self.path, ".draft");
        if !path.try_exists()? {
            return Ok(None);
        }
        crypto::open_draft(&self.header, key, &read(&path)?).map(Some)
    }

    /// Explicitly discard the local form and durably record its removal.
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
