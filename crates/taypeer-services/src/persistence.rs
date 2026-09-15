//! File-backed sessions publish state only after encrypted storage succeeds.

use super::*;

impl From<taypeer_storage::Error> for ServiceError {
    fn from(error: taypeer_storage::Error) -> Self {
        Self::Storage(error)
    }
}

impl DatabaseState {
    pub(super) fn policy(&self) -> taypeer_core::DatabasePolicy {
        self.managed
            .as_ref()
            .map_or_else(Default::default, |managed| managed.policy())
    }
    pub(super) fn path(&self) -> Option<&Path> {
        self.managed
            .as_ref()
            .map(|managed| managed.port.path())
            .or_else(|| self.file.as_ref().map(|file| file.path()))
    }
    pub(super) fn document(&self) -> &Document {
        self.document
            .as_ref()
            .expect("checked sessions own an unlocked document")
    }
    pub(super) fn blobs(&self) -> Result<&BlobStore, ServiceError> {
        self.blobs.as_ref().ok_or(ServiceError::Locked)
    }
    pub(super) fn commit(&mut self, candidate: Document) -> Result<(), ServiceError> {
        if self.document().same_state(&candidate) {
            return Ok(());
        }
        self.commit_blobs(candidate, self.blobs()?.clone())
    }
    pub(super) fn commit_blobs(
        &mut self,
        candidate: Document,
        blobs: BlobStore,
    ) -> Result<(), ServiceError> {
        let refs = candidate.blob_references()?;
        if refs.required.iter().any(|id| blobs.length(id).is_none()) {
            return Err(StorageError::MissingBlob.into());
        }
        let retained = if refs.unknown {
            blobs.clone()
        } else {
            blobs.retained(&refs.retained)
        };
        if let Some(file) = &mut self.file {
            let clear = Zeroizing::new(candidate.export());
            let reader = retained.bundle(&clear)?;
            let length = reader.length();
            file.save_stream(
                self.key.as_ref().ok_or(ServiceError::Locked)?,
                reader,
                length,
            )?;
        }
        if let Some(managed) = &mut self.managed {
            managed.commit(&candidate, &retained)?;
        }
        self.document = Some(candidate);
        let mut keep = refs.retained;
        if let Some(draft) = &self.draft {
            keep.extend(draft.binary_references());
        }
        self.blobs = Some(if refs.unknown {
            blobs
        } else {
            blobs.retained(&keep)
        });
        Ok(())
    }
    pub(super) fn persist_binary_draft(
        &self,
        draft: &DraftState,
        blobs: &BlobStore,
    ) -> Result<(), ServiceError> {
        if let Some(managed) = &self.managed {
            managed.save_draft(draft, blobs)?;
        }
        if let Some(file) = &self.file {
            let mut interrupted = draft.clone();
            interrupted.interrupt();
            let clear = Zeroizing::new(
                serde_json::to_vec(&interrupted).map_err(|_| ServiceError::InvalidDocument)?,
            );
            file.save_binary_draft(
                self.key.as_ref().ok_or(ServiceError::Locked)?,
                &clear,
                &blobs.retained(&draft.binary_references()),
            )?;
        }
        Ok(())
    }
    pub(super) fn change<T>(
        &mut self,
        change: impl FnOnce(&mut Document) -> Result<T, ServiceError>,
    ) -> Result<T, ServiceError> {
        let mut candidate = self.document().clone();
        let result = change(&mut candidate)?;
        self.commit(candidate)?;
        Ok(result)
    }
    pub(super) fn unlock_file(
        &mut self,
        database: &DatabaseId,
        password: &[u8],
        capabilities: &taypeer_core::ClientCapabilities,
    ) -> Result<SessionToken, ServiceError> {
        // Reauthentication must not replace the backing header beneath an active document/key.
        if self.unlocked {
            return Err(ServiceError::InvalidContext);
        }
        let generation = self
            .generation
            .checked_add(1)
            .ok_or(ServiceError::SessionExhausted)?;
        let file = self.file.as_mut().ok_or(ServiceError::InvalidContext)?;
        let (key, clear, mut blobs) = file.unlock_bundle(password)?;
        let document = Document::load(&clear)?;
        let compatibility = capabilities.assess(&document.schema_descriptor()?);
        managed::compatibility::require_read(&compatibility)?;
        if document
            .blob_references()?
            .required
            .iter()
            .any(|id| blobs.length(id).is_none())
        {
            return Err(StorageError::MissingBlob.into());
        }
        if document.database_id() != database {
            return Err(ServiceError::InvalidContext);
        }
        let draft = if compatibility.write.is_supported() {
            load_draft(file, &key, &mut blobs)?
        } else {
            None
        };
        self.blobs = Some(blobs);
        self.document = Some(document);
        self.key = Some(key);
        self.draft = draft;
        self.draft_deferred = !compatibility.write.is_supported();
        self.generation = generation;
        self.unlocked = true;
        Ok(SessionToken {
            database: database.clone(),
            generation,
        })
    }
    pub(super) fn stash_and_close(&mut self) -> Result<(), ServiceError> {
        stash_draft(self);
        if self.managed.is_some() {
            let result = (|| {
                let managed = self.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
                if self.draft_deferred {
                    return Ok(());
                }
                if let Some(draft) = &self.draft {
                    managed.save_draft(draft, self.blobs()?)?;
                } else {
                    managed.port.discard_draft()?;
                }
                Ok(())
            })();
            if let Some(managed) = &mut self.managed {
                managed.close();
            }
            self.document = None;
            self.blobs = None;
            self.draft = None;
            return result;
        }
        let Some(file) = &self.file else {
            return Ok(());
        };
        let result = (|| {
            if self.draft_deferred {
                return Ok(());
            }
            if let Some(draft) = &self.draft {
                self.persist_binary_draft(draft, self.blobs()?)?;
            } else {
                file.discard_draft()?;
            }
            Ok(())
        })();
        // File-backed lock always releases plaintext, even when the draft could not be stored.
        self.document = None;
        self.blobs = None;
        self.key = None;
        self.draft = None;
        result
    }
}

impl DatabaseService {
    /// Create and durably save a file with an exact nonempty master password.
    /// This blocking operation belongs on a worker, never the UI thread.
    pub fn create_file(
        &mut self,
        path: &Path,
        name: String,
        password: &[u8],
    ) -> Result<SessionToken, ServiceError> {
        let document = Document::new(name, (self.clock)())?;
        managed::compatibility::require_write(
            &self.capabilities.assess(&document.schema_descriptor()?),
        )?;
        let clear = Zeroizing::new(document.export());
        let blobs = BlobStore::new()?;
        let reader = blobs.bundle(&clear)?;
        let length = reader.length();
        let (file, key) = FileStore::create_stream(path, password, reader, length, 1000)?;
        self.install_file(document, file, key, None, blobs)
    }
    /// Authenticate and load an existing file, retaining its history and identities.
    /// Unsupported, corrupt or unauthenticated input never replaces an existing session.
    pub fn open_file(
        &mut self,
        path: &Path,
        password: &[u8],
    ) -> Result<SessionToken, ServiceError> {
        let mut file = FileStore::open(path)?;
        let (key, clear, mut blobs) = file.unlock_bundle(password)?;
        let document = Document::load(&clear)?;
        let compatibility = self.capabilities.assess(&document.schema_descriptor()?);
        managed::compatibility::require_read(&compatibility)?;
        if document
            .blob_references()?
            .required
            .iter()
            .any(|id| blobs.length(id).is_none())
        {
            return Err(StorageError::MissingBlob.into());
        }
        let draft = if compatibility.write.is_supported() {
            load_draft(&file, &key, &mut blobs)?
        } else {
            None
        };
        self.install_file(document, file, key, draft, blobs)
    }
    fn install_file(
        &mut self,
        document: Document,
        file: FileStore,
        key: ReadKey,
        draft: Option<DraftState>,
        blobs: BlobStore,
    ) -> Result<SessionToken, ServiceError> {
        let id = document.database_id().clone();
        let draft_deferred = !self
            .capabilities
            .assess(&document.schema_descriptor()?)
            .write
            .is_supported();
        if self.databases.contains_key(&id) {
            return Err(ServiceError::InvalidContext);
        }
        let label = file
            .path()
            .file_name()
            .ok_or(ServiceError::InvalidContext)?
            .to_string_lossy()
            .into_owned();
        self.databases.insert(
            id.clone(),
            DatabaseState {
                document: Some(document),
                blobs: Some(blobs),
                label,
                file: Some(file),
                managed: None,
                key: Some(key),
                generation: 1,
                unlocked: true,
                draft,
                draft_deferred,
            },
        );
        Ok(SessionToken {
            database: id,
            generation: 1,
        })
    }
    /// Whether this catalog item has encrypted file storage rather than volatile demo data.
    pub fn is_file(&self, database: &DatabaseId) -> bool {
        self.databases
            .get(database)
            .is_some_and(|state| state.file.is_some() || state.managed.is_some())
    }
}

fn load_draft(
    file: &FileStore,
    key: &ReadKey,
    blobs: &mut BlobStore,
) -> Result<Option<DraftState>, ServiceError> {
    match file.load_binary_draft(key)? {
        Some(taypeer_storage::BinaryDraft {
            document: bytes,
            blobs: staged,
        }) => {
            let draft: DraftState =
                serde_json::from_slice(&bytes).map_err(|_| ServiceError::InvalidDocument)?;
            if draft
                .binary_references()
                .iter()
                .any(|id| staged.length(id).is_none())
            {
                return Err(StorageError::MissingBlob.into());
            }
            blobs.import(&staged)?;
            Ok(Some(draft))
        }
        None => Ok(None),
    }
}
