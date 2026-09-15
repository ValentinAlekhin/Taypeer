//! Session-checked binary editing, explicit export and quota/retention reporting.

use super::*;
use std::{fs::File, io::Write, path::PathBuf};
use taypeer_core::{
    AttachmentId, BlobId, Color, DatabasePolicy, EntryField, EntryFields, FieldValue, ICON_LIMIT,
    IconRef, IconSource, OperationId,
};
use taypeer_document::SourcePreview;

mod types;
pub use types::*;

impl DatabaseService {
    /// Download favicons for direct group members, optionally including descendants or replacing icons.
    /// Retry checks each durable receipt before making another network request.
    pub fn group_favicons(
        &mut self,
        session: &SessionToken,
        group: &GroupId,
        recursive: bool,
        replace: bool,
        operation: &OperationId,
    ) -> Result<SessionValue<Vec<FaviconResult>>, ServiceError> {
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(ServiceError::EditorAlreadyOpen);
        }
        let groups = state.document().groups()?;
        if !groups.iter().any(|g| &g.id == group) {
            return Err(ServiceError::NotFound);
        }
        let intent =
            serde_json::json!({"group_favicons":group, "recursive":recursive, "replace":replace});
        let mut candidate = state.document().clone();
        candidate.record_binary_operation(operation, &intent)?;
        state.commit(candidate)?;
        let mut selected = std::collections::BTreeSet::from([group.clone()]);
        if recursive {
            loop {
                let count = selected.len();
                for group in &groups {
                    if group
                        .parent
                        .as_ref()
                        .is_some_and(|parent| selected.contains(parent))
                    {
                        selected.insert(group.id.clone());
                    }
                }
                if count == selected.len() {
                    break;
                }
            }
        }
        let entries: Vec<_> = state
            .document()
            .entries()?
            .into_iter()
            .filter(|e| e.group_id.as_ref().is_some_and(|g| selected.contains(g)))
            .collect();
        let mut results = Vec::new();
        for entry in entries {
            let skipped = !replace
                && entry
                    .fields
                    .as_ref()
                    .is_some_and(|f| f.appearance.icon != IconRef::Default);
            let error = if skipped {
                None
            } else {
                let op = OperationId::new(format!(
                    "{}:favicon:{}",
                    operation.as_str(),
                    entry.id.as_str()
                ));
                self.edit_binary(
                    session,
                    &BinaryRequest {
                        target: BinaryTarget::Entry(entry.id.clone()),
                        edit: BinaryEdit::Icon(IconInput::Favicon(None)),
                        review: None,
                    },
                    &op,
                )
                .err()
            };
            results.push(FaviconResult {
                entry: entry.id,
                skipped,
                error,
            });
        }
        Ok(stamped(session, results))
    }
    /// Read binary metadata from an explicit scope without reading or exporting binary contents.
    pub fn binary_view(
        &self,
        session: &SessionToken,
        target: &BinaryTarget,
    ) -> Result<SessionValue<BinaryView>, ServiceError> {
        let state = self.checked(session)?;
        let preview = match target {
            BinaryTarget::Draft => {
                let draft = state
                    .draft
                    .as_ref()
                    .ok_or(ServiceError::NoDraft)?
                    .document()?;
                return Ok(stamped(session, form_view(draft.fields(), state.blobs()?)));
            }
            BinaryTarget::Entry(id) => SourcePreview::Entry(Box::new(state.document().entry(id)?)),
            BinaryTarget::Group(id) => {
                state
                    .document()
                    .groups()?
                    .iter()
                    .find(|g| &g.id == id)
                    .ok_or(ServiceError::NotFound)?;
                let group = state
                    .document()
                    .tree()?
                    .into_iter()
                    .find(|g| g.current && g.address.object == ObjectId::Group(id.clone()))
                    .ok_or(ServiceError::NotFound)?;
                SourcePreview::Group(group)
            }
            BinaryTarget::Revision { entry, revision } => SourcePreview::Entry(Box::new(
                state
                    .document()
                    .history(entry)?
                    .into_iter()
                    .find(|r| &r.id == revision)
                    .ok_or(ServiceError::NotFound)?
                    .snapshot,
            )),
            BinaryTarget::Inspection(target) => {
                super::lifecycle::inspect(state.document(), target)?
            }
        };
        let view = match preview {
            SourcePreview::Group(group) => BinaryView {
                attachments: Vec::new(),
                icons: group.icons,
                foreground: Vec::new(),
                background: Vec::new(),
            },
            SourcePreview::Entry(snapshot) => snapshot_view(&snapshot, state.blobs()?),
        };
        Ok(stamped(session, view))
    }

    /// Stage or confirm an explicit binary action. Failed acquisition leaves previous values intact.
    pub fn edit_binary(
        &mut self,
        session: &SessionToken,
        request: &BinaryRequest,
        operation: &OperationId,
    ) -> Result<SessionValue<()>, ServiceError> {
        if operation.as_str().is_empty() {
            return Err(ServiceError::InvalidInput);
        }
        let intent = serde_json::to_value(request).map_err(|_| ServiceError::InvalidInput)?;
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if !matches!(request.target, BinaryTarget::Draft)
            && state
                .document()
                .binary_receipt(operation, &intent)?
                .is_some()
        {
            return Ok(stamped(session, ()));
        }
        let mut blobs = state.blobs()?.clone();
        match &request.target {
            BinaryTarget::Group(group) => {
                if state.draft.is_some() {
                    return Err(ServiceError::EditorAlreadyOpen);
                }
                let BinaryEdit::Icon(input) = &request.edit else {
                    return Err(ServiceError::InvalidInput);
                };
                state
                    .document()
                    .groups()?
                    .iter()
                    .find(|g| &g.id == group)
                    .ok_or(ServiceError::NotFound)?;
                if request.review.is_none()
                    && state.document().tree()?.iter().any(|node| {
                        node.current
                            && node.address.object == ObjectId::Group(group.clone())
                            && node.icons.len() != 1
                    })
                {
                    return Err(ServiceError::Conflict);
                }
                let icon = acquire_icon(&mut blobs, input, None)?;
                let mut candidate = state.document().clone();
                candidate.set_group_icon(
                    group,
                    icon,
                    request.review.as_deref(),
                    operation,
                    &intent,
                    now,
                )?;
                state.commit_blobs(candidate, blobs)?;
            }
            BinaryTarget::Entry(entry) => {
                if state.draft.is_some() {
                    return Err(ServiceError::EditorAlreadyOpen);
                }
                if request.review.is_some() {
                    return Err(ServiceError::InvalidInput);
                }
                let mut draft = state.document().begin_edit_entry(entry)?;
                apply_edit(&mut draft, &mut blobs, &request.edit, state.policy())?;
                draft
                    .fields()
                    .validate()
                    .map_err(|_| ServiceError::InvalidInput)?;
                check_quota(
                    state.document(),
                    draft.fields(),
                    &blobs,
                    &request.edit,
                    state.policy(),
                )?;
                let mut candidate = state.document().clone();
                candidate.save_binary_entry(draft, operation, &intent, now)?;
                state.commit_blobs(candidate, blobs)?;
            }
            BinaryTarget::Draft => {
                if request.review.is_some() {
                    return Err(ServiceError::InvalidInput);
                }
                let mut draft = state.draft.as_ref().ok_or(ServiceError::NoDraft)?.clone();
                if draft.binary_receipt(operation, &intent)? {
                    return Ok(stamped(session, ()));
                }
                apply_edit(
                    draft.document_mut()?,
                    &mut blobs,
                    &request.edit,
                    state.policy(),
                )?;
                check_quota(
                    state.document(),
                    draft.document()?.fields(),
                    &blobs,
                    &request.edit,
                    state.policy(),
                )?;
                draft.record_binary(operation.clone(), intent);
                state.persist_binary_draft(&draft, &blobs)?;
                state.blobs = Some(blobs);
                state.draft = Some(draft);
            }
            _ => return Err(ServiceError::InvalidInput),
        }
        Ok(stamped(session, ()))
    }

    /// Export an explicitly selected visible content variant to a new file.
    /// A bare blob identity is never sufficient to bypass history or purge visibility.
    pub fn export_binary(
        &self,
        session: &SessionToken,
        target: &BinaryTarget,
        blob: &BlobId,
        destination: &Path,
        overwrite: bool,
    ) -> Result<SessionValue<()>, ServiceError> {
        if let Ok(destination) = destination.canonicalize()
            && self
                .databases
                .values()
                .filter_map(|state| state.path())
                .any(|path| path == destination)
        {
            return Err(ServiceError::InvalidInput);
        }
        let view = self.binary_view(session, target)?.value;
        let allowed = view
            .attachments
            .iter()
            .flat_map(|a| &a.contents)
            .any(|b| &b.id == blob)
            || view.icons.iter().any(|icon| icon.blob() == Some(blob));
        if !allowed {
            return Err(ServiceError::NotFound);
        }
        let state = self.checked(session)?;
        let parent = destination
            .parent()
            .filter(|p| !p.as_os_str().is_empty())
            .unwrap_or(Path::new("."));
        let mut temp = tempfile::NamedTempFile::new_in(parent).map_err(StorageError::from)?;
        std::io::copy(&mut state.blobs()?.reader(blob)?, &mut temp).map_err(StorageError::from)?;
        temp.flush().map_err(StorageError::from)?;
        temp.as_file().sync_all().map_err(StorageError::from)?;
        if overwrite {
            temp.persist(destination).map_err(|_| StorageError::Io)?;
        } else {
            temp.persist_noclobber(destination).map_err(|e| {
                if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                    StorageError::AlreadyExists
                } else {
                    StorageError::Io
                }
            })?;
        }
        File::open(parent)
            .and_then(|dir| dir.sync_all())
            .map_err(|_| StorageError::CommitUncertain)?;
        Ok(stamped(session, ()))
    }

    /// Report product quota separately from binary retention and physical backup storage.
    pub fn storage_usage(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<StorageUsage>, ServiceError> {
        let state = self.checked(session)?;
        let refs = state.document().blob_references()?;
        let blobs = state.blobs()?;
        let attachment_bytes = blobs.unique_bytes(&refs.attachments);
        let all = if refs.unknown {
            blobs.ids().cloned().collect()
        } else {
            refs.retained.clone()
        };
        let (file_bytes, backup_bytes) = physical_usage(state.path())?;
        Ok(stamped(
            session,
            StorageUsage {
                attachment_bytes,
                attachment_limit: state.policy().total_attachment_bytes(),
                over_limit: attachment_bytes > state.policy().total_attachment_bytes(),
                retained_bytes: blobs.unique_bytes(&all),
                draft_bytes: state
                    .draft
                    .as_ref()
                    .map(|d| blobs.unique_bytes(&d.binary_references()))
                    .unwrap_or(0),
                file_bytes,
                backup_bytes,
                missing: refs
                    .retained
                    .into_iter()
                    .filter(|id| blobs.length(id).is_none())
                    .collect(),
                unknown_references: refs.unknown,
            },
        ))
    }

    /// Rebuild retention and atomically omit unreachable binary sections, preserving local drafts.
    pub fn collect_blobs(
        &mut self,
        session: &SessionToken,
        operation: &OperationId,
    ) -> Result<SessionValue<StorageUsage>, ServiceError> {
        let state = self.checked_mut(session)?;
        let intent = serde_json::json!({"action":"collect_blobs"});
        if state
            .document()
            .binary_receipt(operation, &intent)?
            .is_none()
        {
            let mut candidate = state.document().clone();
            candidate.record_binary_operation(operation, &intent)?;
            state.commit_blobs(candidate, state.blobs()?.clone())?;
        }
        self.storage_usage(session)
    }
}

fn acquire_icon(
    blobs: &mut BlobStore,
    input: &IconInput,
    current_url: Option<&str>,
) -> Result<IconRef, ServiceError> {
    let (bytes, source) = match input {
        IconInput::Default => return Ok(IconRef::Default),
        IconInput::Lucide(key) => return Ok(IconRef::Lucide(key.clone())),
        IconInput::File(path) => (
            icons::from_file(path).map_err(ServiceError::Icon)?,
            IconSource::File,
        ),
        IconInput::Url(url) => (
            icons::from_url(url).map_err(ServiceError::Icon)?,
            IconSource::Url(url.clone()),
        ),
        IconInput::Favicon(url) => {
            let url = url
                .as_deref()
                .or(current_url)
                .ok_or(ServiceError::InvalidInput)?;
            (
                icons::favicon(url).map_err(ServiceError::Icon)?,
                IconSource::Favicon(url.to_owned()),
            )
        }
    };
    let blob = blobs.insert(bytes.as_slice(), bytes.len() as u64, ICON_LIMIT)?;
    Ok(IconRef::Image { blob, source })
}

fn stage_file(
    blobs: &mut BlobStore,
    path: &Path,
    policy: DatabasePolicy,
) -> Result<BlobId, ServiceError> {
    let input = File::open(path).map_err(StorageError::from)?;
    let metadata = input.metadata().map_err(StorageError::from)?;
    if !metadata.is_file() {
        return Err(ServiceError::InvalidInput);
    }
    if metadata.len() > policy.attachment_bytes() {
        return Err(ServiceError::AttachmentLimit);
    }
    Ok(blobs.insert(input, metadata.len(), policy.attachment_bytes())?)
}

fn apply_edit(
    draft: &mut taypeer_document::EntryDraft,
    blobs: &mut BlobStore,
    edit: &BinaryEdit,
    policy: DatabasePolicy,
) -> Result<(), ServiceError> {
    match edit {
        BinaryEdit::Icon(input) => {
            let icon = acquire_icon(blobs, input, draft.fields().url.as_deref())?;
            draft.fields_mut().appearance.icon = icon;
        }
        BinaryEdit::Appearance {
            foreground,
            background,
        } => {
            color_update(&mut draft.fields_mut().appearance.foreground, foreground);
            color_update(&mut draft.fields_mut().appearance.background, background);
        }
        BinaryEdit::Attachment(AttachmentEdit::Add { path, name }) => {
            let name = name
                .clone()
                .or_else(|| path.file_name()?.to_str().map(str::to_owned))
                .ok_or(ServiceError::InvalidInput)?;
            if name.is_empty() {
                return Err(ServiceError::InvalidInput);
            }
            let blob = stage_file(blobs, path, policy)?;
            draft.add_attachment(name, blob);
        }
        BinaryEdit::Attachment(AttachmentEdit::Rename { attachment, name }) => {
            if name.is_empty() {
                return Err(ServiceError::InvalidInput);
            }
            draft
                .fields_mut()
                .attachments
                .get_mut(attachment)
                .ok_or(ServiceError::NotFound)?
                .name = name.clone();
        }
        BinaryEdit::Attachment(AttachmentEdit::Replace { attachment, path }) => {
            if !draft.fields().attachments.contains_key(attachment) {
                return Err(ServiceError::NotFound);
            }
            let blob = stage_file(blobs, path, policy)?;
            draft
                .fields_mut()
                .attachments
                .get_mut(attachment)
                .ok_or(ServiceError::NotFound)?
                .blob = blob;
        }
        BinaryEdit::Attachment(AttachmentEdit::Remove { attachment }) => {
            draft
                .fields_mut()
                .attachments
                .remove(attachment)
                .ok_or(ServiceError::NotFound)?;
        }
    }
    Ok(())
}
fn color_update(target: &mut Option<Color>, update: &FieldUpdate<Color>) {
    match update {
        FieldUpdate::Keep => {}
        FieldUpdate::Set(color) => *target = Some(*color),
        FieldUpdate::Clear => *target = None,
    }
}

pub(super) fn check_quota(
    document: &Document,
    fields: &EntryFields,
    blobs: &BlobStore,
    edit: &BinaryEdit,
    policy: DatabasePolicy,
) -> Result<(), ServiceError> {
    if matches!(
        edit,
        BinaryEdit::Attachment(AttachmentEdit::Add { .. } | AttachmentEdit::Replace { .. })
    ) {
        let mut refs = document.blob_references()?.attachments;
        refs.extend(fields.attachments.values().map(|a| a.blob.clone()));
        if blobs.unique_bytes(&refs) > policy.total_attachment_bytes() {
            return Err(ServiceError::AttachmentLimit);
        }
    }
    Ok(())
}

fn form_view(fields: &EntryFields, blobs: &BlobStore) -> BinaryView {
    BinaryView {
        attachments: fields
            .attachments
            .values()
            .map(|a| AttachmentView {
                id: a.id.clone(),
                names: vec![a.name.clone()],
                contents: vec![BlobAvailability {
                    id: a.blob.clone(),
                    bytes: blobs.length(&a.blob),
                }],
                deletion_conflict: false,
            })
            .collect(),
        icons: vec![fields.appearance.icon.clone()],
        foreground: vec![fields.appearance.foreground],
        background: vec![fields.appearance.background],
    }
}
fn snapshot_view(snapshot: &taypeer_core::EntrySnapshot, blobs: &BlobStore) -> BinaryView {
    let mut attachments: BTreeMap<AttachmentId, AttachmentView> = BTreeMap::new();
    let mut result = BinaryView {
        attachments: Vec::new(),
        icons: Vec::new(),
        foreground: Vec::new(),
        background: Vec::new(),
    };
    for state in &snapshot.values {
        for variant in &state.variants {
            match (&state.field, &variant.value) {
                (EntryField::Icon, FieldValue::Icon(icon)) => result.icons.push(icon.clone()),
                (EntryField::Foreground, FieldValue::Color(color)) => {
                    result.foreground.push(*color)
                }
                (EntryField::Background, FieldValue::Color(color)) => {
                    result.background.push(*color)
                }
                (
                    EntryField::AttachmentName(id)
                    | EntryField::AttachmentBlob(id)
                    | EntryField::AttachmentPresence(id),
                    value,
                ) => {
                    let view = attachments
                        .entry(id.clone())
                        .or_insert_with(|| AttachmentView {
                            id: id.clone(),
                            names: Vec::new(),
                            contents: Vec::new(),
                            deletion_conflict: false,
                        });
                    match value {
                        FieldValue::Text(Some(name)) => view.names.push(name.clone()),
                        FieldValue::Blob(blob) => view.contents.push(BlobAvailability {
                            id: blob.clone(),
                            bytes: blobs.length(blob),
                        }),
                        FieldValue::Presence(false) => {
                            view.deletion_conflict = state.variants.len() > 1
                        }
                        _ => {}
                    }
                }
                _ => {}
            }
        }
    }
    result.attachments = attachments
        .into_values()
        .filter(|a| !a.contents.is_empty())
        .collect();
    result
}

fn physical_usage(path: Option<&Path>) -> Result<(u64, u64), ServiceError> {
    let Some(path) = path else {
        return Ok((0, 0));
    };
    let current = std::fs::metadata(path).map_err(StorageError::from)?.len();
    let mut path = path.as_os_str().to_os_string();
    path.push(".backups");
    let directory = PathBuf::from(path);
    let mut backups = 0_u64;
    if directory.try_exists().map_err(StorageError::from)? {
        for entry in std::fs::read_dir(directory).map_err(StorageError::from)? {
            let entry = entry.map_err(StorageError::from)?;
            if entry.path().extension().is_some_and(|e| e == "taypeer") {
                backups = backups
                    .checked_add(entry.metadata().map_err(StorageError::from)?.len())
                    .ok_or(ServiceError::InvalidInput)?;
            }
        }
    }
    Ok((current, backups))
}

#[cfg(test)]
mod tests;
