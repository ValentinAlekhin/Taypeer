//! Authenticate local credentials before a ciphertext host context is activated.
use super::*;
use crate::{RuntimeError, cipher_ipc::storage};
use taypeer_services::{CreateDatabase, DatabaseService};
use taypeer_storage::{ArchiveSeed, ReadKey};

pub(crate) struct PreparedOpen {
    pub seed: Option<ArchiveSeed>,
    pub author: Option<AuthorKey>,
    pub capability: TransportCapability,
    pub authenticated: Option<Digest>,
    pub key: ReadKey,
}

impl NativeProfile {
    pub(crate) fn prepare_open(
        &self,
        path: &Path,
        password: &[u8],
        form: Option<CreateDatabase>,
    ) -> Result<PreparedOpen, RuntimeError> {
        if let Some(form) = form {
            let author = AuthorKey::generate().map_err(|_| RuntimeError::Protocol)?;
            let transport = TransportKey::generate().map_err(|_| RuntimeError::Protocol)?;
            let identity = Identity::new(author.public(), transport.public())
                .map_err(|_| RuntimeError::Protocol)?;
            let now = std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_err(|_| RuntimeError::Protocol)?
                .as_millis();
            let (seed, key) = DatabaseService::prepare_managed_form_with_key_in(
                form,
                password,
                &author,
                identity,
                now.try_into().map_err(|_| RuntimeError::Protocol)?,
                Default::default(),
            )?;
            let head = seed.controls.last().ok_or(RuntimeError::Protocol)?;
            let root = seed
                .controls
                .first()
                .ok_or(RuntimeError::Protocol)?
                .hash()
                .map_err(|_| RuntimeError::Protocol)?;
            let capability = self.create_database_credentials(
                &head.body.database,
                root,
                head.body.epoch,
                &key,
                &author,
                &transport,
            )?;
            return Ok(PreparedOpen {
                seed: Some(seed),
                author: Some(author),
                capability,
                authenticated: None,
                key,
            });
        }
        let snapshot = ArchiveSnapshot::open(path, None).map_err(storage)?;
        let key = DatabaseService::authenticate_archive_credentials(&snapshot, password)?;
        self.bind_join_credentials(&snapshot, password, &key)?;
        let history = DatabaseService::authenticate_archive_keyring_with_key(&snapshot, &key)?;
        let (author, capability) = self.database_credentials(
            &snapshot.chain().head().database,
            snapshot
                .chain()
                .root()
                .map_err(|_| RuntimeError::Protocol)?,
            snapshot.chain().head().epoch,
            &key,
            &history,
        )?;
        Ok(PreparedOpen {
            seed: None,
            author,
            capability,
            authenticated: Some(snapshot.fingerprint()),
            key,
        })
    }
}
