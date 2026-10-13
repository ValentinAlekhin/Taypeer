//! Authentication of the current epoch precedes every local author handshake.
use super::*;
use serde::{Deserialize, Serialize};

/// Content-free binding of a key authenticated by one live plaintext worker.
/// This is an attempt scope, not authority to open an archive or acquire its key.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ManagedKeyScope {
    /// Logical encrypted database.
    pub database: DatabaseId,
    /// Exact registered local working copy.
    pub working_copy: Digest,
    /// Pinned authority root.
    pub root: Digest,
    /// Current read-key epoch; historical keys are never enrollment candidates.
    pub epoch: u64,
    /// Current authenticated authority head.
    pub control: Digest,
    /// Exact ciphertext generation of this attempt.
    pub snapshot: Digest,
    /// Current checkpoint or baseline authenticated before acquiring author authority.
    pub checkpoint: Digest,
}

#[derive(Clone, Copy)]
pub(super) enum AuthenticationProvenance {
    Password { epoch: u64 },
    Key,
}

enum Authentication<'a> {
    Password(&'a [u8]),
    Key(ReadKey),
}
struct AuthenticatedKey {
    key: ReadKey,
    provenance: AuthenticationProvenance,
    measurement: Option<super::policy::PasswordKdfMeasurement>,
}
impl Authentication<'_> {
    fn key(self, selected: &EncryptedObject) -> Result<AuthenticatedKey, ServiceError> {
        match self {
            Self::Password(password) => {
                let (key, measurement) = selected.unlock_key_measured(password)?;
                Ok(AuthenticatedKey {
                    key,
                    provenance: AuthenticationProvenance::Password {
                        epoch: selected.envelope().epoch,
                    },
                    measurement: Some(super::policy::PasswordKdfMeasurement::new(
                        selected,
                        measurement,
                    )?),
                })
            }
            Self::Key(key) => Ok(AuthenticatedKey {
                key,
                provenance: AuthenticationProvenance::Key,
                measurement: None,
            }),
        }
    }
}

fn current_checkpoint(snapshot: &ArchiveSnapshot) -> Result<EncryptedObject, ServiceError> {
    let active = snapshot.object(snapshot.metadata().manifest.body.checkpoint)?;
    if active.envelope().epoch != snapshot.chain().head().epoch
        || active.envelope().control != snapshot.chain().head_hash()?
    {
        latest_baseline(snapshot)
    } else {
        Ok(active)
    }
}

impl DatabaseService {
    /// Inspect the current public key scope of one exact ciphertext generation.
    /// Selection uses the same current-checkpoint/baseline rule as managed opening;
    /// this does not authenticate a key or grant author/read access.
    pub fn archive_key_scope(
        snapshot: &ArchiveSnapshot,
        working_copy: Digest,
    ) -> Result<ManagedKeyScope, ServiceError> {
        compatibility::require_read(
            &snapshot.compatibility(&taypeer_core::ClientCapabilities::default()),
        )?;
        if !snapshot.metadata().journal.forks.is_empty() {
            return Err(taypeer_trust::Error::Fork.into());
        }
        let selected = current_checkpoint(snapshot)?;
        Ok(ManagedKeyScope {
            database: snapshot.chain().head().database.clone(),
            working_copy,
            root: snapshot.chain().root()?,
            epoch: snapshot.chain().head().epoch,
            control: snapshot.chain().head_hash()?,
            snapshot: snapshot.fingerprint(),
            checkpoint: selected.descriptor().digest,
        })
    }

    /// Authenticate only the current checkpoint/baseline with a worker-owned key.
    /// Historical epoch keys are never tried; no author callback or mutation occurs.
    pub fn authenticate_archive_read_key(
        snapshot: &ArchiveSnapshot,
        key: &ReadKey,
    ) -> Result<(), ServiceError> {
        let selected = current_checkpoint(snapshot)?;
        let _ = codec::read_checkpoint(&selected, key, snapshot.chain())?;
        Ok(())
    }

    /// Authenticate a portable archive before acquiring local credentials. The returned
    /// epoch key belongs exclusively to the plaintext worker, never its host.
    pub fn authenticate_archive_credentials(
        snapshot: &ArchiveSnapshot,
        password: &[u8],
    ) -> Result<ReadKey, ServiceError> {
        let selected = current_checkpoint(snapshot)?;
        let key = selected.unlock_key(password)?;
        let _ = codec::read_checkpoint(&selected, &key, snapshot.chain())?;
        Ok(key)
    }

    /// Obtain authenticated historical epoch keys solely for rewrapping local credentials
    /// after a learned rotation. This API is confined to the plaintext worker.
    pub fn authenticate_archive_keyring(
        snapshot: &ArchiveSnapshot,
        password: &[u8],
    ) -> Result<Vec<(u64, ReadKey)>, ServiceError> {
        let selected = current_checkpoint(snapshot)?;
        let key = selected.unlock_key(password)?;
        let (metadata, _) = codec::read_checkpoint(&selected, &key, snapshot.chain())?;
        metadata
            .keys
            .keys()
            .map(|epoch| Ok((*epoch, metadata.key(*epoch)?)))
            .collect()
    }

    /// Authenticate a registered or copied archive. The credential callback runs only
    /// after successful password authentication. An unadmitted identity gets read/export access.
    pub fn open_managed(
        &mut self,
        port: Box<dyn CipherPersistence>,
        password: &[u8],
        author: impl FnOnce() -> Result<Option<AuthorKey>, ServiceError>,
    ) -> Result<SessionToken, ServiceError> {
        self.open_managed_with_author(port, password, |_| author())
    }

    /// Authenticate one immutable generation and pass that exact ciphertext snapshot
    /// to the author capability handshake. The callback must recheck current authority;
    /// ciphertext receipt alone need not invalidate successful authentication.
    pub fn open_managed_with_author(
        &mut self,
        port: Box<dyn CipherPersistence>,
        password: &[u8],
        author: impl FnOnce(&ArchiveSnapshot) -> Result<Option<AuthorKey>, ServiceError>,
    ) -> Result<SessionToken, ServiceError> {
        self.open_with_authentication(port, Authentication::Password(password), author)
    }

    /// Open with a current read key owned by the plaintext worker. This does not acquire
    /// a key from the host: callers transfer its private key-FD result. Current epoch
    /// authentication precedes the author callback, with no historical-key fallback.
    /// Admission, compatibility, blobs and drafts use the same path as password opening.
    pub fn open_managed_with_key_and_author(
        &mut self,
        port: Box<dyn CipherPersistence>,
        key: ReadKey,
        author: impl FnOnce(&ArchiveSnapshot) -> Result<Option<AuthorKey>, ServiceError>,
    ) -> Result<SessionToken, ServiceError> {
        self.open_with_authentication(port, Authentication::Key(key), author)
    }

    fn open_with_authentication(
        &mut self,
        port: Box<dyn CipherPersistence>,
        authentication: Authentication<'_>,
        author: impl FnOnce(&ArchiveSnapshot) -> Result<Option<AuthorKey>, ServiceError>,
    ) -> Result<SessionToken, ServiceError> {
        let snapshot = port.snapshot()?;
        let compatibility = self.capabilities.assess(&snapshot.chain().head().schema);
        compatibility::require_read(&compatibility)?;
        let database = snapshot.chain().head().database.clone();
        let generation = match self.databases.get(&database) {
            None => 1,
            Some(prior)
                if !prior.unlocked
                    && prior.managed.as_ref().is_some_and(|managed| {
                        managed.port.path() == port.path()
                            && managed.port.working_copy() == port.working_copy()
                    }) =>
            {
                prior
                    .generation
                    .checked_add(1)
                    .ok_or(ServiceError::InvalidContext)?
            }
            Some(_) => return Err(ServiceError::InvalidContext),
        };
        let active = snapshot.object(snapshot.metadata().manifest.body.checkpoint)?;
        let mut selected = current_checkpoint(&snapshot)?;
        let AuthenticatedKey {
            key,
            provenance: authentication,
            measurement: password_kdf,
        } = authentication.key(&selected)?;
        let (mut metadata, mut document) =
            codec::read_checkpoint(&selected, &key, snapshot.chain())?;
        let author = author(&snapshot)?;
        let admitted = author
            .as_ref()
            .is_some_and(|a| snapshot.chain().head().members.contains_key(&a.device_id()));
        if admitted
            && selected.envelope().kind != ObjectKind::Baseline
            && author
                .as_ref()
                .is_some_and(|a| a.device_id() != selected.envelope().author)
        {
            selected = latest_baseline(&snapshot)?;
            (metadata, document) = codec::read_checkpoint(
                &selected,
                &metadata.key(selected.envelope().epoch)?,
                snapshot.chain(),
            )?;
        }
        let own_checkpoint = author
            .as_ref()
            .is_some_and(|a| a.device_id() == active.envelope().author);
        if selected.descriptor().digest != active.descriptor().digest
            && (own_checkpoint || !admitted)
        {
            let (prior, accepted) = codec::read_checkpoint(
                &active,
                &metadata.key(active.envelope().epoch)?,
                snapshot.chain(),
            )?;
            document.merge(&accepted)?;
            metadata = apply::merge_accepted_metadata(metadata, &prior)?;
        }
        if !own_checkpoint {
            // Portable receipt claims from another device are never local apply/discard decisions.
            metadata.processed.clear();
            metadata.discarded.clear();
            metadata.discarded_sources.clear();
        }
        if let Some(author) = author
            .as_ref()
            .filter(|_| admitted && compatibility.write.is_supported())
        {
            document.set_writer(*author.device_id().as_bytes());
        } else {
            document.clear_writer();
        }
        let baseline = if selected.envelope().kind == ObjectKind::Baseline {
            selected.descriptor().digest
        } else {
            snapshot.metadata().manifest.body.baseline
        };
        let mut blobs = BlobStore::with_temporary_storage(snapshot.temporary_storage())?;
        let managed = ManagedState {
            port,
            baseline,
            accepted: selected.descriptor().digest,
            reconciled_application: false,
            snapshot,
            capabilities: self.capabilities.clone(),
            pending_management: None,
            open: Some(Unlocked {
                author,
                header: selected.password_header()?,
                metadata,
                control: selected.envelope().control,
                authentication,
                password_kdf,
            }),
        };
        managed.load_required_blobs(&document, &mut blobs)?;
        // An unsupported writer must not decode/rewrite a possibly newer local draft.
        let mut draft = if compatibility.write.is_supported() {
            managed.load_draft(&mut blobs)?
        } else {
            DraftCollection::new(database.clone())
        };
        draft.reconcile(&document)?;
        if draft.is_empty() && compatibility.write.is_supported() {
            managed.port.discard_draft()?;
        }
        let label = managed
            .port
            .path()
            .file_name()
            .map(|name| name.to_string_lossy().into_owned())
            // Descriptor-backed workers retain only a public identity while locked.
            .unwrap_or_else(|| database.to_string());
        self.databases.insert(
            database.clone(),
            DatabaseState {
                document: Some(document),
                blobs: Some(blobs),
                label,
                file: None,
                managed: Some(managed),
                key: None,
                generation,
                unlocked: true,
                drafts: draft,
                draft_deferred: !compatibility.write.is_supported(),
                draft_maintenance_pending: false,
                write_uncertain: false,
                pending_icon: None,
            },
        );
        Ok(SessionToken {
            database,
            generation,
        })
    }

    /// Obtain the current enrollment key solely inside the plaintext worker. A keyed
    /// unlock never counts as password provenance, and a rotation requires a new master
    /// unlock. Locked, stale, forked or uncertain projections do not yield a key.
    /// The caller must keep this result out of host/UI DTOs and general credential ports.
    pub fn password_authenticated_key(
        &self,
        session: &SessionToken,
    ) -> Result<(ManagedKeyScope, ReadKey), ServiceError> {
        let (scope, key, _) = self.password_authenticated_key_source(session)?;
        Ok((scope, key))
    }

    /// Verify an enrollment FD result against this still password-authenticated scope.
    /// A changed generation/epoch/provenance is refused before testing the supplied key.
    /// The proof writes nothing and never requests an author credential.
    pub fn verify_password_authenticated_read_key(
        &self,
        session: &SessionToken,
        expected: &ManagedKeyScope,
        supplied: &ReadKey,
    ) -> Result<(), ServiceError> {
        let (scope, _key, current) = self.password_authenticated_key_source(session)?;
        if &scope != expected {
            return Err(StorageError::Changed.into());
        }
        Self::authenticate_archive_read_key(&current, supplied)
    }

    fn password_authenticated_key_source(
        &self,
        session: &SessionToken,
    ) -> Result<(ManagedKeyScope, ReadKey, ArchiveSnapshot), ServiceError> {
        let state = self.checked(session)?;
        let managed = state.managed.as_ref().ok_or(ServiceError::InvalidContext)?;
        if state.write_uncertain
            || state.pending_icon.is_some()
            || managed.pending_management.is_some()
        {
            return Err(StorageError::CommitUncertain.into());
        }
        let open = managed.open.as_ref().ok_or(ServiceError::Locked)?;
        let current = managed.port.snapshot()?;
        if !current.metadata().journal.forks.is_empty() {
            return Err(taypeer_trust::Error::Fork.into());
        }
        let epoch = current.chain().head().epoch;
        match open.authentication {
            AuthenticationProvenance::Password {
                epoch: authenticated,
            } if authenticated == epoch => {}
            AuthenticationProvenance::Password { .. } | AuthenticationProvenance::Key => {
                return Err(ServiceError::Unauthorized);
            }
        }
        let control = current.chain().head_hash()?;
        if current.fingerprint() != managed.snapshot.fingerprint() || open.control != control {
            return Err(StorageError::Changed.into());
        }
        let scope = Self::archive_key_scope(&current, managed.port.working_copy())?;
        let key = open.metadata.key(epoch)?;
        Self::authenticate_archive_read_key(&current, &key)?;
        Ok((scope, key, current))
    }
}
