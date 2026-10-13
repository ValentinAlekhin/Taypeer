//! Bounded reads and the existing file/directory durability boundary.
use super::{
    RegistryError,
    types::{Binding, MAX_RECORD_BYTES, Record},
};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use taypeer_storage::{PublicationMode, publish_file};

pub(super) trait Publisher: Send + Sync {
    fn publish(&self, destination: &Path, bytes: &[u8]) -> Result<(), RegistryError>;
}
pub(super) struct FilePublisher;
impl Publisher for FilePublisher {
    fn publish(&self, destination: &Path, bytes: &[u8]) -> Result<(), RegistryError> {
        let directory = destination.parent().ok_or(RegistryError::Io)?;
        let mut temporary =
            tempfile::NamedTempFile::new_in(directory).map_err(|_| RegistryError::Io)?;
        temporary.write_all(bytes).map_err(|_| RegistryError::Io)?;
        publish_file(temporary, destination, PublicationMode::Replace).map_err(
            |error| match error {
                taypeer_storage::Error::CommitUncertain => RegistryError::CommitUncertain,
                _ => RegistryError::Io,
            },
        )
    }
}
pub(super) struct Loaded {
    pub(super) record: Record,
    pub(super) bytes: Vec<u8>,
}
pub(super) fn load(path: &Path, binding: &Binding) -> Result<Option<Loaded>, RegistryError> {
    let file = match File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(RegistryError::Io),
    };
    let mut bytes = Vec::new();
    file.take(MAX_RECORD_BYTES as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| RegistryError::Io)?;
    let record = Record::decode(&bytes, binding)?;
    Ok(Some(Loaded { record, bytes }))
}
