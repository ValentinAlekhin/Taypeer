//! File-backed sessions publish state only after encrypted storage succeeds.

use super::*;

impl From<taypeer_storage::Error> for ServiceError {
    fn from(error: taypeer_storage::Error) -> Self {
        Self::Storage(error)
    }
}

impl DatabaseState {
    pub(super) fn document(&self) -> &Document {
        self.document
            .as_ref()
            .expect("checked sessions own an unlocked document")
    }
    pub(super) fn commit(&mut self, candidate: Document) -> Result<(), ServiceError> {
        if self.document().same_state(&candidate) {
            return Ok(());
        }
        if let Some(file) = &mut self.file {
            let clear = Zeroizing::new(candidate.export());
            let key = self.key.as_ref().ok_or(ServiceError::Locked)?;
            file.save(key, &clear)?;
        }
        self.document = Some(candidate);
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
        let (key, clear) = file.unlock(password)?;
        let document = Document::load(&clear)?;
        if document.database_id() != database {
            return Err(ServiceError::InvalidContext);
        }
        let draft = file
            .load_draft(&key)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| ServiceError::InvalidDocument))
            .transpose()?;
        self.document = Some(document);
        self.key = Some(key);
        self.draft = draft;
        self.generation = generation;
        self.unlocked = true;
        Ok(SessionToken {
            database: database.clone(),
            generation,
        })
    }
    pub(super) fn stash_and_close(&mut self) -> Result<(), ServiceError> {
        stash_draft(self);
        let Some(file) = &self.file else {
            return Ok(());
        };
        let result = (|| {
            if let Some(draft) = &self.draft {
                let clear = Zeroizing::new(
                    serde_json::to_vec(draft).map_err(|_| ServiceError::InvalidDocument)?,
                );
                file.save_draft(self.key.as_ref().ok_or(ServiceError::Locked)?, &clear)?;
            } else {
                file.discard_draft()?;
            }
            Ok(())
        })();
        // File-backed lock always releases plaintext, even when the draft could not be stored.
        self.document = None;
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
        let clear = Zeroizing::new(document.export());
        let (file, key) = FileStore::create(path, password, &clear)?;
        self.install_file(document, file, key, None)
    }
    /// Authenticate and load an existing file, retaining its history and identities.
    /// Unsupported, corrupt or unauthenticated input never replaces an existing session.
    pub fn open_file(
        &mut self,
        path: &Path,
        password: &[u8],
    ) -> Result<SessionToken, ServiceError> {
        let mut file = FileStore::open(path)?;
        let (key, clear) = file.unlock(password)?;
        let document = Document::load(&clear)?;
        let draft = file
            .load_draft(&key)?
            .map(|bytes| serde_json::from_slice(&bytes).map_err(|_| ServiceError::InvalidDocument))
            .transpose()?;
        self.install_file(document, file, key, draft)
    }
    fn install_file(
        &mut self,
        document: Document,
        file: FileStore,
        key: ReadKey,
        draft: Option<DraftState>,
    ) -> Result<SessionToken, ServiceError> {
        let id = document.database_id().clone();
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
                label,
                file: Some(file),
                key: Some(key),
                generation: 1,
                unlocked: true,
                draft,
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
            .is_some_and(|state| state.file.is_some())
    }
}
