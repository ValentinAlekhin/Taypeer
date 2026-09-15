//! Private callbacks from the plaintext worker to its one ciphertext writer.
use crate::{
    RuntimeError,
    protocol::{Response, read_frame, write_frame},
};
use serde::{Deserialize, Serialize};
use std::{
    collections::BTreeSet,
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::{Arc, Mutex},
};
use taypeer_storage::{
    ArchiveJournal, ArchiveSeed, ArchiveSnapshot, CipherPersistence, EncryptedObject,
    PreparedCommit,
};
use taypeer_trust::{ControlChain, Digest, SignedControl};
use tempfile::NamedTempFile;

#[derive(Serialize, Deserialize)]
pub(crate) enum WorkerMessage {
    Response(Response),
    Io(Box<IoRequest>),
}
#[derive(Serialize, Deserialize)]
pub(crate) enum IoRequest {
    Open,
    Create(Seed),
    Recover { path: PathBuf, seed: Seed },
    Snapshot { known: Option<Digest> },
    Commit(Commit),
    SaveDraft(PathBuf),
    LoadDraft,
    DiscardDraft,
}
#[derive(Serialize, Deserialize)]
pub(crate) enum IoValue {
    Snapshot {
        spool: PathBuf,
        fingerprint: Digest,
        root: Digest,
        working_copy: Digest,
    },
    Unchanged,
    Draft(Option<PathBuf>),
    Done,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct IoReply {
    pub result: Result<IoValue, RuntimeError>,
}

#[derive(Serialize, Deserialize)]
pub(crate) struct Seed {
    pub controls: Vec<SignedControl>,
    pub objects: Vec<PathBuf>,
    pub checkpoint: Digest,
    pub baseline: Digest,
}
#[derive(Serialize, Deserialize)]
pub(crate) struct Commit {
    pub expected: Digest,
    pub control: Digest,
    pub controls: Vec<SignedControl>,
    pub objects: Vec<PathBuf>,
    pub remove: BTreeSet<Digest>,
    pub checkpoint: Digest,
    pub baseline: Digest,
    pub journal: ArchiveJournal,
}
pub(crate) fn read_objects(
    paths: Vec<PathBuf>,
    directory: &Path,
    chain: &ControlChain,
) -> Result<Vec<EncryptedObject>, RuntimeError> {
    paths
        .into_iter()
        .map(|path| {
            let path = checked_spool(&path, directory)?;
            EncryptedObject::open(&path, chain).map_err(storage)
        })
        .collect()
}
pub(crate) fn checked_spool(path: &Path, directory: &Path) -> Result<PathBuf, RuntimeError> {
    let path = path.canonicalize().map_err(|_| RuntimeError::Transport)?;
    if path.parent() != Some(directory) || !path.is_file() {
        return Err(RuntimeError::Protocol);
    }
    Ok(path)
}
pub(crate) fn storage(error: taypeer_storage::Error) -> RuntimeError {
    taypeer_services::ServiceError::Storage(error).into()
}
fn port_error(error: RuntimeError) -> taypeer_storage::Error {
    match error {
        RuntimeError::Service(taypeer_services::ServiceError::Storage(error)) => error,
        RuntimeError::Service(taypeer_services::ServiceError::Trust(error)) => error.into(),
        _ => taypeer_storage::Error::Io,
    }
}

/// The serve loop and synchronous persistence callbacks share one private channel.
/// A command's read lock is released before dispatch; callbacks cannot recursively borrow it.
pub(crate) struct Channel {
    reader: Box<dyn Read + Send>,
    writer: Box<dyn Write + Send>,
}
impl Channel {
    pub fn new(reader: impl Read + Send + 'static, writer: impl Write + Send + 'static) -> Self {
        Self {
            reader: Box::new(reader),
            writer: Box::new(writer),
        }
    }
    pub fn read<T: serde::de::DeserializeOwned>(&mut self) -> Result<T, RuntimeError> {
        read_frame(&mut self.reader)
    }
    pub fn response(
        &mut self,
        result: Result<serde_json::Value, RuntimeError>,
    ) -> Result<(), RuntimeError> {
        write_frame(
            &mut self.writer,
            &WorkerMessage::Response(Response { result }),
        )
    }
    pub fn io(&mut self, request: IoRequest) -> Result<IoValue, RuntimeError> {
        write_frame(&mut self.writer, &WorkerMessage::Io(Box::new(request)))?;
        self.read::<IoReply>()?.result
    }
}
pub(crate) struct RemotePersistence {
    channel: Arc<Mutex<Channel>>,
    path: PathBuf,
    directory: PathBuf,
    working_copy: Digest,
    cached: Mutex<ArchiveSnapshot>,
}
impl RemotePersistence {
    pub fn attach(
        channel: Arc<Mutex<Channel>>,
        path: PathBuf,
        directory: PathBuf,
        seed: Option<ArchiveSeed>,
    ) -> Result<Self, RuntimeError> {
        let mut spools = Vec::new();
        let request = match seed {
            Some(seed) => IoRequest::Create(Seed {
                controls: seed.controls,
                checkpoint: seed.checkpoint,
                baseline: seed.baseline,
                objects: spool_objects(seed.objects, &directory, &mut spools)?,
            }),
            None => IoRequest::Open,
        };
        let value = channel
            .lock()
            .map_err(|_| RuntimeError::Transport)?
            .io(request)?;
        let (snapshot, working_copy) = read_snapshot(value, &directory)?;
        Ok(Self {
            channel,
            path,
            directory,
            working_copy,
            cached: Mutex::new(snapshot),
        })
    }
    fn io(&self, request: IoRequest) -> Result<IoValue, taypeer_storage::Error> {
        self.channel
            .lock()
            .map_err(|_| taypeer_storage::Error::Io)?
            .io(request)
            .map_err(port_error)
    }
    fn accept(&self, value: IoValue) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        let mut cached = self.cached.lock().map_err(|_| taypeer_storage::Error::Io)?;
        if matches!(value, IoValue::Unchanged) {
            return Ok(cached.clone());
        }
        let (snapshot, working_copy) = read_snapshot(value, &self.directory).map_err(port_error)?;
        if working_copy != self.working_copy {
            return Err(taypeer_storage::Error::Changed);
        }
        *cached = snapshot.clone();
        Ok(snapshot)
    }
}
fn read_snapshot(
    value: IoValue,
    directory: &Path,
) -> Result<(ArchiveSnapshot, Digest), RuntimeError> {
    let IoValue::Snapshot {
        spool,
        fingerprint,
        root,
        working_copy,
    } = value
    else {
        return Err(RuntimeError::Protocol);
    };
    let path = checked_spool(&spool, directory)?;
    let snapshot = ArchiveSnapshot::open(&path, Some(root)).map_err(storage)?;
    if snapshot.fingerprint() != fingerprint {
        return Err(RuntimeError::Protocol);
    }
    Ok((snapshot, working_copy))
}
pub(crate) fn spool_objects(
    objects: Vec<EncryptedObject>,
    directory: &Path,
    spools: &mut Vec<NamedTempFile>,
) -> Result<Vec<PathBuf>, RuntimeError> {
    objects
        .into_iter()
        .map(|object| {
            let mut file = NamedTempFile::new_in(directory).map_err(|_| RuntimeError::Transport)?;
            std::io::copy(&mut object.reader().map_err(storage)?, &mut file)
                .map_err(|_| RuntimeError::Transport)?;
            let path = file.path().to_owned();
            spools.push(file);
            Ok(path)
        })
        .collect()
}
impl CipherPersistence for RemotePersistence {
    fn snapshot(&self) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        let known = self
            .cached
            .lock()
            .map_err(|_| taypeer_storage::Error::Io)?
            .fingerprint();
        self.accept(self.io(IoRequest::Snapshot { known: Some(known) })?)
    }
    fn commit(&self, request: PreparedCommit) -> Result<ArchiveSnapshot, taypeer_storage::Error> {
        let mut spools = Vec::new();
        let objects =
            spool_objects(request.objects, &self.directory, &mut spools).map_err(port_error)?;
        let request = Commit {
            expected: request.expected,
            control: request.control,
            controls: request.controls,
            objects,
            remove: request.remove,
            checkpoint: request.checkpoint,
            baseline: request.baseline,
            journal: request.journal,
        };
        self.accept(self.io(IoRequest::Commit(request))?)
    }
    fn working_copy(&self) -> Digest {
        self.working_copy
    }
    fn save_draft(&self, object: &EncryptedObject) -> Result<(), taypeer_storage::Error> {
        let mut spools = Vec::new();
        let paths = spool_objects(vec![object.clone()], &self.directory, &mut spools)
            .map_err(port_error)?;
        let path = paths
            .into_iter()
            .next()
            .ok_or(taypeer_storage::Error::InvalidFile)?;
        match self.io(IoRequest::SaveDraft(path))? {
            IoValue::Done => Ok(()),
            _ => Err(taypeer_storage::Error::InvalidFile),
        }
    }
    fn load_draft(
        &self,
        chain: &ControlChain,
    ) -> Result<Option<EncryptedObject>, taypeer_storage::Error> {
        match self.io(IoRequest::LoadDraft)? {
            IoValue::Draft(None) => Ok(None),
            IoValue::Draft(Some(path)) => {
                let path = checked_spool(&path, &self.directory).map_err(port_error)?;
                EncryptedObject::open(&path, chain).map(Some)
            }
            _ => Err(taypeer_storage::Error::InvalidFile),
        }
    }
    fn discard_draft(&self) -> Result<(), taypeer_storage::Error> {
        match self.io(IoRequest::DiscardDraft)? {
            IoValue::Done => Ok(()),
            _ => Err(taypeer_storage::Error::InvalidFile),
        }
    }
    fn path(&self) -> &Path {
        &self.path
    }
}
