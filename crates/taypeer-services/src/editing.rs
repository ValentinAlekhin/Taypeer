//! Service-owned causal editors, immutable save attempts and their local collection.

use crate::*;
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use taypeer_core::{
    BlobId, DatabaseMetadataPatch, DraftId, GroupMetadataPatch, IconRef, IconSource, OperationId,
};

// One aggregate budget for durable local operation records, not a per-form limit.
const MAX_LOCAL_OPERATIONS: usize = 100_000;

/// Monotonic input revision within one local draft, independent of document history.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DraftRevision(pub u64);

/// Nonsecret object address of a local editor.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftTarget {
    /// Existing entry.
    Entry(EntryId),
    /// Unsaved entry with a reserved identity and owning user-created group.
    NewEntry {
        /// Reserved entry identity.
        entry: EntryId,
        /// Required user-created destination group.
        group: GroupId,
    },
    /// Existing group.
    Group(GroupId),
    /// Unsaved group in an optional parent.
    NewGroup {
        /// Reserved local identity; replaced by the confirmed object identity on save.
        group: GroupId,
        /// Parent, absent for the root.
        parent: Option<GroupId>,
    },
    /// Name and description of the authenticated database.
    Database,
}

/// Exact form snapshot identity; session identity is supplied by SessionValue.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftIdentity {
    /// Local editor identity, preserved through lock and restart.
    pub draft: DraftId,
    /// Object to which the editor belongs.
    pub target: DraftTarget,
    /// Captured input revision.
    pub revision: DraftRevision,
}

/// List projection deliberately containing no entered names or secret values.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DraftSummary {
    /// Stable editor and input revision.
    pub identity: DraftIdentity,
    /// Whether local input differs from its last confirmed baseline.
    pub dirty: bool,
}

/// Durability of the exact captured snapshot; errors are returned separately.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DraftSaveOutcome {
    /// The document, history and receipt were durably committed.
    Saved {
        /// Identity captured before the commit.
        identity: DraftIdentity,
        /// Stable idempotency key for that snapshot.
        operation: OperationId,
    },
    /// No changed input required a document revision.
    Unchanged {
        /// Exact inspected form.
        identity: DraftIdentity,
        /// Stable idempotency key.
        operation: OperationId,
    },
}

/// Ordinary descriptive form values; protected entry contents use EditorView instead.
#[derive(Clone, Serialize, Deserialize)]
pub struct MetadataDraftView {
    /// Exact input identity.
    pub identity: DraftIdentity,
    /// Entered name, including unfinished empty input.
    pub name: String,
    /// Exact optional description.
    pub description: Option<String>,
    /// Group icon, absent for database metadata.
    pub icon: Option<IconRef>,
    /// Whether the input differs from its confirmed baseline.
    pub dirty: bool,
    /// Exact group-form parent; None is top level. Database metadata has no parent control.
    pub parent: Option<GroupId>,
    /// Valid destinations with labels from the form's original causal context.
    pub parent_candidates: Vec<GroupParentCandidate>,
}

/// One valid group-form destination; no content or permission is inferred by the UI.
#[derive(Clone, Serialize, Deserialize)]
pub struct GroupParentCandidate {
    /// Stable group identity.
    pub id: GroupId,
    /// Exact name from the form basis.
    pub name: String,
}

impl std::fmt::Debug for MetadataDraftView {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("MetadataDraftView([REDACTED])")
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) struct MetadataFields {
    name: String,
    description: Option<String>,
    icon: Option<IconRef>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum MetadataField {
    Name,
    Description,
    Icon,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
struct MetadataParent {
    baseline: Option<GroupId>,
    current: Option<GroupId>,
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct MetadataDraft {
    identity: DraftIdentity,
    base: Vec<String>,
    baseline: MetadataFields,
    fields: MetadataFields,
    changed: BTreeSet<MetadataField>,
    // Empty receipts must keep the serialized shape of legacy pending Save
    // snapshots: their fingerprint covers this entire captured draft.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    binary_receipts: BTreeMap<OperationId, serde_json::Value>,
    // Never hydrate legacy captured drafts: their typed JSON owns a pending receipt.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    parent: Option<MetadataParent>,
}

impl MetadataDraft {
    fn new(
        target: DraftTarget,
        base: Vec<String>,
        fields: MetadataFields,
        parent: Option<MetadataParent>,
    ) -> Result<Self, ServiceError> {
        Ok(Self {
            identity: DraftIdentity {
                draft: new_draft_id()?,
                target,
                revision: DraftRevision::default(),
            },
            base,
            baseline: fields.clone(),
            fields,
            changed: BTreeSet::new(),
            binary_receipts: BTreeMap::new(),
            parent,
        })
    }
    fn parent(&self, document: &Document) -> Result<Option<GroupId>, ServiceError> {
        if let Some(parent) = &self.parent {
            return Ok(parent.current.clone());
        }
        match &self.identity.target {
            DraftTarget::Group(group) => Ok(document.group_parent_at(group, &self.base)?),
            DraftTarget::NewGroup { parent, .. } => Ok(parent.clone()),
            DraftTarget::Database => Ok(None),
            _ => Err(ServiceError::InvalidContext),
        }
    }
    fn is_dirty(&self) -> bool {
        self.fields != self.baseline
            || self
                .parent
                .as_ref()
                .is_some_and(|parent| parent.current != parent.baseline)
    }
    fn view(&self, document: &Document) -> Result<MetadataDraftView, ServiceError> {
        let group = match &self.identity.target {
            DraftTarget::Group(group) => Some(group),
            DraftTarget::NewGroup { .. } => None,
            DraftTarget::Database => {
                return Ok(MetadataDraftView {
                    identity: self.identity.clone(),
                    name: self.fields.name.clone(),
                    description: self.fields.description.clone(),
                    icon: self.fields.icon.clone(),
                    dirty: self.is_dirty(),
                    parent: None,
                    parent_candidates: Vec::new(),
                });
            }
            _ => return Err(ServiceError::InvalidContext),
        };
        Ok(MetadataDraftView {
            identity: self.identity.clone(),
            name: self.fields.name.clone(),
            description: self.fields.description.clone(),
            icon: self.fields.icon.clone(),
            dirty: self.is_dirty(),
            parent: self.parent(document)?,
            parent_candidates: document
                .group_parent_candidates(group, &self.base)?
                .into_iter()
                .map(|group| GroupParentCandidate {
                    id: group.id,
                    name: group.name,
                })
                .collect(),
        })
    }
    fn update(&mut self, fields: MetadataFields) -> Result<(), ServiceError> {
        self.update_with_parent(fields, self.parent.clone())
    }
    fn update_with_parent(
        &mut self,
        fields: MetadataFields,
        parent: Option<MetadataParent>,
    ) -> Result<(), ServiceError> {
        if fields == self.fields && parent == self.parent {
            return Ok(());
        }
        self.identity.revision.0 = self
            .identity
            .revision
            .0
            .checked_add(1)
            .ok_or(ServiceError::InvalidContext)?;
        self.fields = fields;
        self.parent = parent;
        self.changed.clear();
        if self.fields.name != self.baseline.name {
            self.changed.insert(MetadataField::Name);
        }
        if self.fields.description != self.baseline.description {
            self.changed.insert(MetadataField::Description);
        }
        if self.fields.icon != self.baseline.icon {
            self.changed.insert(MetadataField::Icon);
        }
        Ok(())
    }
    fn binary_references(&self) -> BTreeSet<BlobId> {
        [&self.baseline.icon, &self.fields.icon]
            .into_iter()
            .filter_map(|icon| icon.as_ref().and_then(IconRef::blob).cloned())
            .collect()
    }
    pub(super) fn icon(&self) -> Result<IconRef, ServiceError> {
        self.fields.icon.clone().ok_or(ServiceError::InvalidContext)
    }
    fn binary_receipt(
        &self,
        operation: &OperationId,
        intent: &serde_json::Value,
    ) -> Result<bool, ServiceError> {
        match self.binary_receipts.get(operation) {
            Some(old) if old == intent => Ok(true),
            Some(_) => Err(ServiceError::InvalidInput),
            None => Ok(false),
        }
    }
    pub(super) fn stage_icon(
        &mut self,
        icon: IconRef,
        operation: OperationId,
        intent: serde_json::Value,
    ) -> Result<(), ServiceError> {
        if !matches!(
            self.identity.target,
            DraftTarget::Group(_) | DraftTarget::NewGroup { .. }
        ) {
            return Err(ServiceError::InvalidContext);
        }
        let previous = self.identity.revision;
        let mut fields = self.fields.clone();
        fields.icon = Some(icon);
        self.update(fields)?;
        if self.identity.revision == previous {
            self.identity.revision.0 = previous
                .0
                .checked_add(1)
                .ok_or(ServiceError::InvalidContext)?;
        }
        self.binary_receipts.insert(operation, intent);
        Ok(())
    }
    fn save_command(
        &self,
        document: &mut Document,
        now: i64,
        receipt: &taypeer_document::CommandReceipt<'_>,
    ) -> Result<serde_json::Value, ServiceError> {
        let name = if self.changed.contains(&MetadataField::Name) {
            FieldUpdate::Set(self.fields.name.clone())
        } else {
            FieldUpdate::Keep
        };
        let description = if self.changed.contains(&MetadataField::Description) {
            self.fields
                .description
                .clone()
                .map_or(FieldUpdate::Clear, FieldUpdate::Set)
        } else {
            FieldUpdate::Keep
        };
        match &self.identity.target {
            DraftTarget::Database => {
                document.patch_metadata_command(
                    &DatabaseMetadataPatch { name, description },
                    &self.base,
                    now,
                    Some(receipt),
                )?;
                Ok(serde_json::Value::Null)
            }
            DraftTarget::Group(id) => {
                let icon = if self.changed.contains(&MetadataField::Icon) {
                    self.fields
                        .icon
                        .clone()
                        .map_or(FieldUpdate::Clear, FieldUpdate::Set)
                } else {
                    FieldUpdate::Keep
                };
                let group = document.update_group_metadata_command(
                    id,
                    &GroupMetadataPatch {
                        name,
                        description,
                        icon,
                        parent: self
                            .parent
                            .as_ref()
                            .filter(|parent| parent.current != parent.baseline)
                            .map_or(FieldUpdate::Keep, |parent| {
                                parent
                                    .current
                                    .clone()
                                    .map_or(FieldUpdate::Clear, FieldUpdate::Set)
                            }),
                    },
                    &self.base,
                    now,
                    Some(receipt),
                )?;
                serde_json::to_value(group).map_err(|_| ServiceError::InvalidDocument)
            }
            DraftTarget::NewGroup { group, .. } => {
                let parent = self.parent(document)?;
                document.validate_group_parent(None, parent.as_ref(), &self.base)?;
                let group = document.create_group_metadata_with_id_command(
                    group.clone(),
                    self.fields.name.clone(),
                    self.fields.description.clone(),
                    self.fields.icon.clone().unwrap_or_default(),
                    parent,
                    now,
                    Some(receipt),
                )?;
                serde_json::to_value(group).map_err(|_| ServiceError::InvalidDocument)
            }
            _ => Err(ServiceError::InvalidContext),
        }
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) enum CapturedDraft {
    Entry(Box<DraftState>),
    Metadata(Box<MetadataDraft>),
}

impl CapturedDraft {
    pub(super) fn icon(&self) -> Result<IconRef, ServiceError> {
        match self {
            Self::Entry(draft) => Ok(draft.document()?.fields().appearance.icon.clone()),
            Self::Metadata(draft) => draft.icon(),
        }
    }
    fn baseline_icon(&self) -> Option<&IconRef> {
        match self {
            Self::Entry(draft) => Some(draft.baseline_icon()),
            Self::Metadata(draft) => draft.baseline.icon.as_ref(),
        }
    }
    pub(super) fn stage_icon(
        &mut self,
        icon: IconRef,
        operation: OperationId,
        intent: serde_json::Value,
    ) -> Result<(), ServiceError> {
        match self {
            Self::Entry(draft) => {
                draft.document_mut()?.fields_mut().appearance.icon = icon;
                draft.record_binary(operation, intent)
            }
            Self::Metadata(draft) => draft.stage_icon(icon, operation, intent),
        }
    }
    pub(super) fn identity(&self) -> DraftIdentity {
        match self {
            Self::Entry(draft) => draft.identity(),
            Self::Metadata(draft) => draft.identity.clone(),
        }
    }
    pub(super) fn binary_receipt(
        &self,
        operation: &OperationId,
        intent: &serde_json::Value,
    ) -> Result<bool, ServiceError> {
        match self {
            Self::Entry(draft) => draft.binary_receipt(operation, intent),
            Self::Metadata(draft) => draft.binary_receipt(operation, intent),
        }
    }
    fn is_dirty(&self) -> bool {
        match self {
            Self::Entry(draft) => draft.is_dirty(),
            Self::Metadata(draft) => draft.is_dirty(),
        }
    }

    fn validate_for_confirmation(&self, document: &Document) -> Result<(), ServiceError> {
        let valid = match self {
            Self::Entry(draft) => draft.can_save(),
            Self::Metadata(draft) => !draft.fields.name.is_empty(),
        };
        if !valid {
            return Err(ServiceError::InvalidInput);
        }
        if let Self::Metadata(draft) = self {
            match &draft.identity.target {
                DraftTarget::NewGroup { .. } => document
                    .validate_group_parent(None, draft.parent(document)?.as_ref(), &draft.base)
                    .map_err(operations::candidate_input_error)?,
                DraftTarget::Group(group)
                    if draft
                        .parent
                        .as_ref()
                        .is_some_and(|parent| parent.current != parent.baseline) =>
                {
                    document
                        .validate_group_parent(
                            Some(group),
                            draft.parent(document)?.as_ref(),
                            &draft.base,
                        )
                        .map_err(operations::candidate_input_error)?;
                }
                _ => {}
            }
        }
        Ok(())
    }
}

#[derive(Clone, Serialize, Deserialize)]
pub(super) struct SnapshotAttempt {
    identity: DraftIdentity,
    fingerprint: serde_json::Value,
    captured: Option<CapturedDraft>,
    outcome: Option<DraftSaveOutcome>,
}

/// Receipt-only ownership preserves replay without retaining a stale clean editor.
#[derive(Clone, Serialize, Deserialize)]
struct FormIconReceipt {
    identity: DraftIdentity,
    intent: IconReceiptIntent,
    icon: IconRef,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(super) enum IconReceiptIntent {
    Import(IconSource),
    StoredFavicon(BlobId),
}

/// Each form is owned once: the current entry, parked entries or metadata forms.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct DraftCollection {
    version: u16,
    database: DatabaseId,
    pub(super) entry: Option<DraftState>,
    entries: BTreeMap<DraftId, DraftState>,
    metadata: BTreeMap<DraftId, MetadataDraft>,
    active_metadata: Option<DraftId>,
    attempts: BTreeMap<OperationId, SnapshotAttempt>,
    #[serde(default)]
    cancelled: BTreeSet<OperationId>,
    // One explicit OS picker may retain an otherwise clean form through lock.
    #[serde(default)]
    pinned: Option<DraftId>,
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    imported_icons: BTreeMap<OperationId, FormIconReceipt>,
}

impl DraftCollection {
    pub(super) fn active_binary_form(&self, id: &DraftId) -> bool {
        self.active_metadata.as_ref().map_or_else(
            || {
                self.entry
                    .as_ref()
                    .is_some_and(|draft| &draft.identity().draft == id)
            },
            |active| active == id,
        )
    }
    pub(super) fn new(database: DatabaseId) -> Self {
        Self {
            version: 2,
            database,
            entry: None,
            entries: BTreeMap::new(),
            metadata: BTreeMap::new(),
            active_metadata: None,
            attempts: BTreeMap::new(),
            cancelled: BTreeSet::new(),
            pinned: None,
            imported_icons: BTreeMap::new(),
        }
    }
    pub(super) fn validate(&self, database: &DatabaseId) -> Result<(), ServiceError> {
        if self.version != 2 {
            return Err(ServiceError::ReadCompatibility);
        }
        if &self.database != database
            || self.entries.len() + self.metadata.len() + usize::from(self.entry.is_some()) > 10_000
            || self.local_operation_count() > MAX_LOCAL_OPERATIONS
        {
            return Err(ServiceError::InvalidContext);
        }
        if self.pinned.as_ref().is_some_and(|id| !self.contains(id)) {
            return Err(ServiceError::InvalidDocument);
        }
        for (id, draft) in &self.entries {
            if id != &draft.identity().draft {
                return Err(ServiceError::InvalidDocument);
            }
        }
        for (id, draft) in &self.metadata {
            if id != &draft.identity.draft
                || self.entries.contains_key(id)
                || !matches!(
                    draft.identity.target,
                    DraftTarget::Group(_) | DraftTarget::NewGroup { .. } | DraftTarget::Database
                )
            {
                return Err(ServiceError::InvalidDocument);
            }
        }
        if self.entry.as_ref().is_some_and(|entry| {
            self.entries.contains_key(&entry.identity().draft)
                || self.metadata.contains_key(&entry.identity().draft)
        }) || self
            .active_metadata
            .as_ref()
            .is_some_and(|id| !self.metadata.contains_key(id))
        {
            return Err(ServiceError::InvalidDocument);
        }
        for (operation, attempt) in &self.attempts {
            if self.cancelled.contains(operation) {
                return Err(ServiceError::InvalidDocument);
            }
            match (&attempt.captured, &attempt.outcome) {
                (Some(captured), None) => {
                    let identity = match captured {
                        CapturedDraft::Entry(draft) => draft.identity(),
                        CapturedDraft::Metadata(draft) => draft.identity.clone(),
                    };
                    if identity != attempt.identity
                        || commands::fingerprint(&(&identity, captured))? != attempt.fingerprint
                    {
                        return Err(ServiceError::InvalidDocument);
                    }
                }
                (None, Some(outcome)) => {
                    let (identity, acknowledged) = match outcome {
                        DraftSaveOutcome::Saved {
                            identity,
                            operation,
                        }
                        | DraftSaveOutcome::Unchanged {
                            identity,
                            operation,
                        } => (identity, operation),
                    };
                    if identity != &attempt.identity || acknowledged != operation {
                        return Err(ServiceError::InvalidDocument);
                    }
                }
                _ => return Err(ServiceError::InvalidDocument),
            }
        }
        for receipt in self.imported_icons.values() {
            if !matches!(
                receipt.identity.target,
                DraftTarget::Entry(_)
                    | DraftTarget::NewEntry { .. }
                    | DraftTarget::Group(_)
                    | DraftTarget::NewGroup { .. }
            ) {
                return Err(ServiceError::InvalidDocument);
            }
            match (&receipt.intent, &receipt.icon) {
                (
                    IconReceiptIntent::Import(source),
                    IconRef::Image {
                        source: original, ..
                    },
                ) if source == original => {}
                (
                    IconReceiptIntent::StoredFavicon(identity),
                    IconRef::Image {
                        blob,
                        source: IconSource::Favicon(_),
                    },
                ) if identity == blob => {}
                _ => return Err(ServiceError::InvalidDocument),
            }
        }
        Ok(())
    }
    fn local_operation_count(&self) -> usize {
        self.attempts
            .len()
            .saturating_add(self.cancelled.len())
            .saturating_add(self.imported_icons.len())
    }
    pub(super) fn require_icon_capacity(&self) -> Result<(), ServiceError> {
        if self.local_operation_count() >= MAX_LOCAL_OPERATIONS {
            Err(ServiceError::InvalidContext)
        } else {
            Ok(())
        }
    }
    pub(super) fn form_icon_receipt(
        &self,
        draft: &DraftId,
        intent: &IconReceiptIntent,
        operation: &OperationId,
    ) -> Result<Option<DraftIdentity>, ServiceError> {
        let Some(receipt) = self.imported_icons.get(operation) else {
            return Ok(None);
        };
        if &receipt.identity.draft != draft || &receipt.intent != intent {
            return Err(ServiceError::InvalidInput);
        }
        let identity = match self.captured(draft) {
            Ok(current) => current.identity(),
            Err(ServiceError::NoDraft) => receipt.identity.clone(),
            Err(error) => return Err(error),
        };
        Ok(Some(identity))
    }
    pub(super) fn record_icon_operation(
        &mut self,
        identity: DraftIdentity,
        intent: IconReceiptIntent,
        icon: IconRef,
        operation: OperationId,
    ) -> Result<(), ServiceError> {
        self.require_icon_capacity()?;
        if self.imported_icons.contains_key(&operation) {
            return Err(ServiceError::InvalidInput);
        }
        self.imported_icons.insert(
            operation,
            FormIconReceipt {
                identity,
                intent,
                icon,
            },
        );
        Ok(())
    }
    pub(super) fn stored_favicon_icon(
        &self,
        id: &DraftId,
    ) -> Result<Option<IconRef>, ServiceError> {
        let draft = self.captured(id)?;
        let current = draft.icon()?;
        if is_favicon(&current) {
            return Ok(Some(current));
        }
        if let Some(receipt) = self
            .imported_icons
            .values()
            .filter(|receipt| &receipt.identity.draft == id && is_favicon(&receipt.icon))
            .max_by_key(|receipt| receipt.identity.revision)
        {
            return Ok(Some(receipt.icon.clone()));
        }
        Ok(draft
            .baseline_icon()
            .filter(|icon| is_favicon(icon))
            .cloned())
    }
    pub(super) fn database_id(&self) -> &DatabaseId {
        &self.database
    }
    fn contains(&self, id: &DraftId) -> bool {
        self.entry
            .as_ref()
            .is_some_and(|entry| &entry.identity().draft == id)
            || self.entries.contains_key(id)
            || self.metadata.contains_key(id)
    }
    pub(super) fn clear_entry(&mut self) {
        if let Some(entry) = self.entry.take()
            && self.pinned.as_ref() == Some(&entry.identity().draft)
        {
            self.pinned = None;
        }
    }
    fn remove_unpinned_clean_parked(&mut self) {
        self.entries
            .retain(|id, entry| entry.is_dirty() || self.pinned.as_ref() == Some(id));
        self.metadata.retain(|id, draft| {
            draft.is_dirty()
                || self.pinned.as_ref() == Some(id)
                || self.active_metadata.as_ref() == Some(id)
        });
    }
    pub(super) fn park_entry(&mut self) {
        if let Some(draft) = self.entry.take()
            && (draft.is_dirty() || self.pinned.as_ref() == Some(&draft.identity().draft))
        {
            self.entries.insert(draft.identity().draft, draft);
        }
    }
    pub(super) fn activate_entry(&mut self, draft: DraftState) {
        self.park_entry();
        self.active_metadata = None;
        self.entry = Some(draft);
    }
    pub(super) fn take_entry_for(&mut self, id: &EntryId) -> Option<DraftState> {
        let key = self
            .entries
            .iter()
            .find(|(_, draft)| draft.entry_id() == Some(id))
            .map(|(id, _)| id.clone())?;
        self.entries.remove(&key)
    }
    pub(super) fn binary_references(&self) -> BTreeSet<BlobId> {
        let mut references = BTreeSet::new();
        for draft in self.entry.iter().chain(self.entries.values()) {
            references.extend(draft.binary_references());
        }
        for draft in self.metadata.values() {
            references.extend(draft.binary_references());
        }
        for attempt in self
            .attempts
            .values()
            .filter(|attempt| attempt.outcome.is_none())
        {
            references.extend(match &attempt.captured {
                Some(CapturedDraft::Entry(draft)) => draft.binary_references(),
                Some(CapturedDraft::Metadata(draft)) => draft.binary_references(),
                None => BTreeSet::new(),
            });
        }
        references.extend(
            self.imported_icons
                .values()
                .filter_map(|receipt| receipt.icon.blob().cloned()),
        );
        references
    }
    pub(super) fn interrupt(&mut self) {
        if let Some(entry) = &mut self.entry {
            entry.interrupt();
        }
        for entry in self.entries.values_mut() {
            entry.interrupt();
        }
    }
    pub(super) fn stash(&mut self) {
        if self.entry.as_ref().is_some_and(|entry| {
            !entry.is_dirty() && self.pinned.as_ref() != Some(&entry.identity().draft)
        }) {
            self.entry = None;
        }
        self.entries
            .retain(|id, entry| entry.is_dirty() || self.pinned.as_ref() == Some(id));
        self.metadata
            .retain(|id, draft| draft.is_dirty() || self.pinned.as_ref() == Some(id));
        if self
            .active_metadata
            .as_ref()
            .is_some_and(|id| !self.metadata.contains_key(id))
        {
            self.active_metadata = None;
        }
        self.interrupt();
    }
    pub(super) fn is_empty(&self) -> bool {
        self.entry.is_none()
            && self.entries.is_empty()
            && self.metadata.is_empty()
            && self.attempts.is_empty()
            && self.cancelled.is_empty()
            && self.imported_icons.is_empty()
    }

    pub(super) fn reconcile(&mut self, document: &Document) -> Result<(), ServiceError> {
        if self
            .entry
            .as_ref()
            .map(|draft| draft.confirmed(document))
            .transpose()?
            .unwrap_or(false)
        {
            // Legacy SaveDraft closes its editor; the receipt already protects its data.
            self.clear_entry();
        }
        let operations: Vec<_> = self
            .attempts
            .iter()
            .filter(|(_, attempt)| attempt.outcome.is_none())
            .map(|(operation, _)| operation.clone())
            .collect();
        for operation in operations {
            let attempt = self
                .attempts
                .get(&operation)
                .ok_or(ServiceError::InvalidDocument)?
                .clone();
            let receipt = document
                .command_receipt(&operation, "save_draft_snapshot")
                .map_err(|error| match error {
                    taypeer_document::Error::DuplicateId => ServiceError::OperationConflict,
                    other => other.into(),
                })?;
            if let Some((fingerprint, result)) = receipt {
                if fingerprint != attempt.fingerprint {
                    return Err(ServiceError::OperationConflict);
                }
                self.finish_snapshot(&operation, document, &result)?;
            }
        }
        Ok(())
    }

    pub(super) fn captured(&self, id: &DraftId) -> Result<CapturedDraft, ServiceError> {
        if let Some(entry) = self
            .entry
            .as_ref()
            .filter(|entry| &entry.identity().draft == id)
        {
            let mut entry = entry.clone();
            entry.resume();
            return Ok(CapturedDraft::Entry(Box::new(entry)));
        }
        if let Some(entry) = self.entries.get(id) {
            let mut entry = entry.clone();
            entry.resume();
            return Ok(CapturedDraft::Entry(Box::new(entry)));
        }
        self.metadata
            .get(id)
            .cloned()
            .map(|draft| CapturedDraft::Metadata(Box::new(draft)))
            .ok_or(ServiceError::NoDraft)
    }

    /// Replace only an existing exact form without changing which editor is active.
    pub(super) fn replace_binary_form(&mut self, draft: CapturedDraft) -> Result<(), ServiceError> {
        let id = draft.identity().draft;
        match draft {
            CapturedDraft::Entry(draft) => {
                if self
                    .entry
                    .as_ref()
                    .is_some_and(|entry| entry.identity().draft == id)
                {
                    self.entry = Some(*draft);
                } else {
                    let current = self.entries.get_mut(&id).ok_or(ServiceError::NoDraft)?;
                    *current = *draft;
                }
            }
            CapturedDraft::Metadata(draft) => {
                let current = self.metadata.get_mut(&id).ok_or(ServiceError::NoDraft)?;
                *current = *draft;
            }
        }
        Ok(())
    }

    fn finish_snapshot(
        &mut self,
        operation: &OperationId,
        document: &Document,
        result: &serde_json::Value,
    ) -> Result<DraftSaveOutcome, ServiceError> {
        let attempt = self
            .attempts
            .get(operation)
            .ok_or(ServiceError::InvalidDocument)?
            .clone();
        let captured = attempt.captured.ok_or(ServiceError::InvalidDocument)?;
        match captured {
            CapturedDraft::Entry(saved) => {
                let current = self
                    .entry
                    .as_mut()
                    .filter(|entry| entry.identity().draft == attempt.identity.draft)
                    .or_else(|| self.entries.get_mut(&attempt.identity.draft));
                if let Some(current) = current {
                    current.continue_from(&saved, document)?;
                }
            }
            CapturedDraft::Metadata(saved) => {
                if let Some(current) = self.metadata.get_mut(&attempt.identity.draft) {
                    current.base = document.command_heads(operation, "save_draft_snapshot")?;
                    current.baseline = saved.fields.clone();
                    if matches!(saved.identity.target, DraftTarget::NewGroup { .. }) {
                        let group: taypeer_core::Group = serde_json::from_value(result.clone())
                            .map_err(|_| ServiceError::InvalidDocument)?;
                        current.identity.target = DraftTarget::Group(group.id);
                    }
                    if let Some(parent) = &mut current.parent {
                        let DraftTarget::Group(group) = &current.identity.target else {
                            return Err(ServiceError::InvalidDocument);
                        };
                        parent.baseline = document.group_parent_at(group, &current.base)?;
                    }
                    let fields = current.fields.clone();
                    current.changed.clear();
                    if fields.name != current.baseline.name {
                        current.changed.insert(MetadataField::Name);
                    }
                    if fields.description != current.baseline.description {
                        current.changed.insert(MetadataField::Description);
                    }
                    if fields.icon != current.baseline.icon {
                        current.changed.insert(MetadataField::Icon);
                    }
                }
            }
        }
        let outcome = DraftSaveOutcome::Saved {
            identity: attempt.identity,
            operation: operation.clone(),
        };
        let attempt = self
            .attempts
            .get_mut(operation)
            .ok_or(ServiceError::InvalidDocument)?;
        attempt.outcome = Some(outcome.clone());
        attempt.captured = None;
        Ok(outcome)
    }
}

fn is_favicon(icon: &IconRef) -> bool {
    matches!(
        icon,
        IconRef::Image {
            source: IconSource::Favicon(_),
            ..
        }
    )
}

pub(super) fn new_draft_id() -> Result<DraftId, ServiceError> {
    Ok(DraftId::new(crate::new_operation_id()?.as_str()))
}

impl DatabaseService {
    /// Durably retain the exact active form through an intentional OS picker
    /// background transition, including a clean editor. This creates no document
    /// revision and does not change dirty state or input revision. A new explicit
    /// selection replaces an orphaned pin while preserving every dirty form.
    pub fn pin_active_form(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<DraftIdentity>, ServiceError> {
        let identity = self
            .active_draft(session)?
            .value
            .ok_or(ServiceError::NoDraft)?;
        let state = self.checked_mut(session)?;
        if let Some(entry) = &state.drafts.entry {
            entry.document()?;
        }
        let mut collection = state.drafts.clone();
        collection.pinned = Some(identity.draft.clone());
        collection.remove_unpinned_clean_parked();
        collection.validate(&session.database)?;
        state.persist_draft_collection(&collection, state.blobs()?)?;
        state.drafts = collection;
        Ok(stamped(session, identity))
    }

    /// Release this picker pin durably after success or cancellation. Clean forms
    /// may leave local storage; the active in-memory editor and dirty forms remain.
    /// Repeating an already completed unpin succeeds without changing another pin.
    pub fn unpin_form(
        &mut self,
        session: &SessionToken,
        id: &DraftId,
    ) -> Result<SessionValue<()>, ServiceError> {
        let state = self.checked_mut(session)?;
        if state.drafts.pinned.as_ref() != Some(id) {
            return Ok(stamped(session, ()));
        }
        let mut collection = state.drafts.clone();
        collection.pinned = None;
        let mut retained = collection.clone();
        retained.stash();
        state.persist_draft_collection(&retained, state.blobs()?)?;
        state.drafts = collection;
        Ok(stamped(session, ()))
    }

    /// Confirm durable local storage of every current form and immutable pending attempt.
    /// This creates no document revision and does not confirm a failed database write.
    pub fn persist_drafts(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<()>, ServiceError> {
        self.checked(session)?;
        let state = self
            .databases
            .get_mut(&session.database)
            .ok_or(ServiceError::NotFound)?;
        if state.pending_icon.is_some() {
            return Err(StorageError::CommitUncertain.into());
        }
        if let Some(managed) = &state.managed {
            managed.check_edit_permission()?;
        }
        let mut collection = state.drafts.clone();
        collection.stash();
        collection.validate(&session.database)?;
        state.persist_draft_collection(&collection, state.blobs()?)?;
        state.draft_maintenance_pending = false;
        Ok(stamped(session, ()))
    }

    /// Read the exact active form identity without exposing entered values.
    pub fn active_draft(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Option<DraftIdentity>>, ServiceError> {
        let collection = &self.checked(session)?.drafts;
        let identity = match &collection.active_metadata {
            Some(id) => Some(
                collection
                    .metadata
                    .get(id)
                    .ok_or(ServiceError::InvalidDocument)?
                    .identity
                    .clone(),
            ),
            None => collection.entry.as_ref().map(DraftState::identity),
        };
        Ok(stamped(session, identity))
    }

    /// List recoverable local forms without returning names or entered values.
    pub fn drafts(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Vec<DraftSummary>>, ServiceError> {
        let collection = &self.checked(session)?.drafts;
        let mut summaries: Vec<_> = collection
            .entry
            .iter()
            .chain(collection.entries.values())
            .filter(|draft| {
                draft.is_dirty() || collection.pinned.as_ref() == Some(&draft.identity().draft)
            })
            .map(|draft| DraftSummary {
                identity: draft.identity(),
                dirty: draft.is_dirty(),
            })
            .collect();
        summaries.extend(
            collection
                .metadata
                .values()
                .filter(|draft| {
                    draft.is_dirty() || collection.pinned.as_ref() == Some(&draft.identity.draft)
                })
                .map(|draft| DraftSummary {
                    identity: draft.identity.clone(),
                    dirty: draft.is_dirty(),
                }),
        );
        summaries.sort_by(|left, right| left.identity.draft.cmp(&right.identity.draft));
        Ok(stamped(session, summaries))
    }

    /// Activate a retained form. Runtime must request its masked view separately.
    pub fn resume_draft(
        &mut self,
        session: &SessionToken,
        id: &DraftId,
    ) -> Result<SessionValue<DraftIdentity>, ServiceError> {
        let state = self.checked_mut(session)?;
        if state
            .drafts
            .entry
            .as_ref()
            .is_some_and(|draft| &draft.identity().draft == id)
        {
            let draft = state.drafts.entry.as_mut().ok_or(ServiceError::NoDraft)?;
            draft.resume();
            return Ok(stamped(session, draft.identity()));
        }
        if let Some(mut draft) = state.drafts.entries.remove(id) {
            draft.resume();
            let identity = draft.identity();
            state.drafts.activate_entry(draft);
            return Ok(stamped(session, identity));
        }
        let identity = state
            .drafts
            .metadata
            .get(id)
            .ok_or(ServiceError::NoDraft)?
            .identity
            .clone();
        state.drafts.park_entry();
        state.drafts.active_metadata = Some(id.clone());
        Ok(stamped(session, identity))
    }

    /// Durably remove only the selected local form, never its confirmed document object.
    pub fn delete_draft(
        &mut self,
        session: &SessionToken,
        id: &DraftId,
    ) -> Result<SessionValue<()>, ServiceError> {
        // Discard may follow an uncertain save. Read its receipt without retrying
        // the write; a snapshot that did not commit must remain discarded.
        self.reconcile_uncertain(session)?;
        let state = self.checked_mut(session)?;
        let mut collection = state.drafts.clone();
        let active = collection
            .entry
            .as_ref()
            .is_some_and(|draft| &draft.identity().draft == id);
        let removed = if active {
            collection.entry.take().is_some()
        } else {
            collection.entries.remove(id).is_some() || collection.metadata.remove(id).is_some()
        };
        let before = collection.imported_icons.len();
        collection
            .imported_icons
            .retain(|_, receipt| &receipt.identity.draft != id);
        let removed_receipt = collection.imported_icons.len() != before;
        if !removed && !removed_receipt {
            return Err(ServiceError::NoDraft);
        }
        if collection.pinned.as_ref() == Some(id) {
            collection.pinned = None;
        }
        if collection.active_metadata.as_ref() == Some(id) {
            collection.active_metadata = None;
        }
        // checked_mut excludes uncertain publication. Any exact durable receipt
        // must be reconciled before deletion; known failed attempts can be cancelled.
        let operations: Vec<_> = collection
            .attempts
            .iter()
            .filter(|(_, attempt)| &attempt.identity.draft == id && attempt.outcome.is_none())
            .map(|(operation, _)| operation.clone())
            .collect();
        for operation in operations {
            if state
                .document()
                .command_receipt(&operation, "save_draft_snapshot")?
                .is_some()
            {
                return Err(ServiceError::OperationConflict);
            }
            collection.attempts.remove(&operation);
            collection.cancelled.insert(operation);
        }
        state.persist_draft_collection(&collection, state.blobs()?)?;
        state.drafts = collection;
        Ok(stamped(session, ()))
    }

    /// Begin or automatically continue the local causal form for a group.
    pub fn start_edit_group(
        &mut self,
        session: &SessionToken,
        id: &GroupId,
    ) -> Result<SessionValue<MetadataDraftView>, ServiceError> {
        self.start_metadata_draft(session, DraftTarget::Group(id.clone()))
    }

    /// Begin an unfinished new group, reserving its identity without creating an object.
    pub fn start_create_group(
        &mut self,
        session: &SessionToken,
        parent: Option<GroupId>,
    ) -> Result<SessionValue<MetadataDraftView>, ServiceError> {
        let group = GroupId::new(crate::new_operation_id()?.as_str());
        self.start_metadata_draft(session, DraftTarget::NewGroup { group, parent })
    }

    /// Begin or automatically continue name/description editing at the original heads.
    pub fn start_edit_database_info(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<MetadataDraftView>, ServiceError> {
        self.start_metadata_draft(session, DraftTarget::Database)
    }

    fn start_metadata_draft(
        &mut self,
        session: &SessionToken,
        target: DraftTarget,
    ) -> Result<SessionValue<MetadataDraftView>, ServiceError> {
        let state = self.checked_mut(session)?;
        if let Some(id) = state
            .drafts
            .metadata
            .iter()
            .find(|(_, draft)| draft.identity.target == target)
            .map(|(id, _)| id.clone())
        {
            state.drafts.park_entry();
            state.drafts.active_metadata = Some(id.clone());
            return Ok(stamped(
                session,
                state
                    .drafts
                    .metadata
                    .get(&id)
                    .ok_or(ServiceError::NoDraft)?
                    .view(state.document())?,
            ));
        }
        let fields = match &target {
            DraftTarget::Database => MetadataFields {
                name: state.document().display_name()?,
                description: state.document().description()?,
                icon: None,
            },
            DraftTarget::Group(id) => {
                let group = state
                    .document()
                    .groups()?
                    .into_iter()
                    .find(|group| &group.id == id)
                    .ok_or(ServiceError::NotFound)?;
                MetadataFields {
                    name: group.name,
                    description: state.document().group_description(id)?,
                    icon: Some(group.icon),
                }
            }
            DraftTarget::NewGroup { parent, .. } => {
                if let Some(parent) = parent
                    && !state
                        .document()
                        .groups()?
                        .iter()
                        .any(|group| &group.id == parent)
                {
                    return Err(ServiceError::NotFound);
                }
                MetadataFields {
                    name: String::new(),
                    description: None,
                    icon: Some(IconRef::default()),
                }
            }
            _ => return Err(ServiceError::InvalidContext),
        };
        let base = state.document().heads();
        let parent = match &target {
            DraftTarget::Group(group) => Some(state.document().group_parent_at(group, &base)?),
            DraftTarget::NewGroup { parent, .. } => Some(parent.clone()),
            _ => None,
        }
        .map(|parent| MetadataParent {
            baseline: parent.clone(),
            current: parent,
        });
        let draft = MetadataDraft::new(target, base, fields, parent)?;
        let view = draft.view(state.document())?;
        state.drafts.park_entry();
        state.drafts.active_metadata = Some(draft.identity.draft.clone());
        state
            .drafts
            .metadata
            .insert(draft.identity.draft.clone(), draft);
        Ok(stamped(session, view))
    }

    /// Read ordinary descriptive form values, preserving exact empty/absent input.
    pub fn metadata_draft(
        &self,
        session: &SessionToken,
        id: &DraftId,
    ) -> Result<SessionValue<MetadataDraftView>, ServiceError> {
        let state = self.checked(session)?;
        let draft = state.drafts.metadata.get(id).ok_or(ServiceError::NoDraft)?;
        Ok(stamped(session, draft.view(state.document())?))
    }

    /// Apply only addressed group fields; original heads are preserved until a confirmed save.
    pub fn patch_group_draft(
        &mut self,
        session: &SessionToken,
        id: &DraftId,
        patch: GroupMetadataPatch,
    ) -> Result<SessionValue<DraftIdentity>, ServiceError> {
        let state = self.checked_mut(session)?;
        if let FieldUpdate::Set(icon) = &patch.icon
            && let Some(blob) = icon.blob()
            && state.blobs()?.length(blob).is_none()
        {
            return Err(ServiceError::NotFound);
        }
        let draft = state.drafts.metadata.get(id).ok_or(ServiceError::NoDraft)?;
        if !matches!(
            draft.identity.target,
            DraftTarget::Group(_) | DraftTarget::NewGroup { .. }
        ) {
            return Err(ServiceError::InvalidContext);
        }
        let selected = match &patch.parent {
            FieldUpdate::Keep => None,
            FieldUpdate::Set(parent) => Some(Some(parent.clone())),
            FieldUpdate::Clear => Some(None),
        };
        let parent = match selected {
            None => draft.parent.clone(),
            Some(selected) => {
                let group = match &draft.identity.target {
                    DraftTarget::Group(group) => Some(group),
                    _ => None,
                };
                state
                    .document()
                    .validate_group_parent(group, selected.as_ref(), &draft.base)
                    .map_err(operations::candidate_input_error)?;
                let previous = draft.parent(state.document())?;
                if selected == previous {
                    draft.parent.clone()
                } else {
                    Some(MetadataParent {
                        baseline: draft
                            .parent
                            .as_ref()
                            .map(|parent| parent.baseline.clone())
                            .unwrap_or(previous),
                        current: selected,
                    })
                }
            }
        };
        let draft = state
            .drafts
            .metadata
            .get_mut(id)
            .ok_or(ServiceError::NoDraft)?;
        let mut fields = draft.fields.clone();
        apply_metadata_name(&mut fields.name, patch.name);
        apply_metadata_optional(&mut fields.description, patch.description);
        match patch.icon {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(icon) => fields.icon = Some(icon),
            FieldUpdate::Clear => fields.icon = Some(IconRef::default()),
        }
        draft.update_with_parent(fields, parent)?;
        Ok(stamped(session, draft.identity.clone()))
    }

    /// Apply only addressed database fields, retaining unfinished names in local state.
    pub fn patch_database_draft(
        &mut self,
        session: &SessionToken,
        id: &DraftId,
        patch: DatabaseMetadataPatch,
    ) -> Result<SessionValue<DraftIdentity>, ServiceError> {
        let state = self.checked_mut(session)?;
        let draft = state
            .drafts
            .metadata
            .get_mut(id)
            .ok_or(ServiceError::NoDraft)?;
        if draft.identity.target != DraftTarget::Database {
            return Err(ServiceError::InvalidContext);
        }
        let mut fields = draft.fields.clone();
        apply_metadata_name(&mut fields.name, patch.name);
        apply_metadata_optional(&mut fields.description, patch.description);
        draft.update(fields)?;
        Ok(stamped(session, draft.identity.clone()))
    }

    /// Save an immutable captured revision, preserving the editor and all later input.
    /// Invalid/unfinished values, including untouched new forms, are rejected
    /// without closing the editor or reserving the operation identity.
    /// Retries use the same operation and revision; changed intent is rejected.
    pub fn save_draft_snapshot(
        &mut self,
        session: &SessionToken,
        id: &DraftId,
        revision: DraftRevision,
        operation: &OperationId,
    ) -> Result<SessionValue<DraftSaveOutcome>, ServiceError> {
        self.reconcile_uncertain(session)?;
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if state.drafts.cancelled.contains(operation) {
            return Err(ServiceError::OperationConflict);
        }
        if let Some(attempt) = state.drafts.attempts.get(operation) {
            if &attempt.identity.draft != id || attempt.identity.revision != revision {
                return Err(ServiceError::OperationConflict);
            }
            if let Some(outcome) = &attempt.outcome {
                let outcome = outcome.clone();
                if state.draft_maintenance_pending
                    && state
                        .persist_draft_collection(&state.drafts, state.blobs()?)
                        .is_ok()
                {
                    state.draft_maintenance_pending = false;
                }
                return Ok(stamped(session, outcome));
            }
        } else {
            if state.drafts.local_operation_count() >= MAX_LOCAL_OPERATIONS {
                return Err(ServiceError::InvalidContext);
            }
            let captured = state.drafts.captured(id)?;
            let identity = match &captured {
                CapturedDraft::Entry(draft) => draft.identity(),
                CapturedDraft::Metadata(draft) => draft.identity.clone(),
            };
            if identity.revision != revision {
                return Err(ServiceError::InvalidContext);
            }
            captured.validate_for_confirmation(state.document())?;
            let fingerprint = commands::fingerprint(&(&identity, &captured))?;
            // Check the portable namespace even for unchanged local attempts.
            if state
                .document()
                .command_receipt(operation, "save_draft_snapshot")
                .map_err(|error| match error {
                    taypeer_document::Error::DuplicateId => ServiceError::OperationConflict,
                    other => other.into(),
                })?
                .is_some()
            {
                return Err(ServiceError::OperationConflict);
            }
            state.drafts.attempts.insert(
                operation.clone(),
                SnapshotAttempt {
                    identity,
                    fingerprint,
                    captured: Some(captured),
                    outcome: None,
                },
            );
        }
        let attempt = state
            .drafts
            .attempts
            .get(operation)
            .ok_or(ServiceError::InvalidDocument)?
            .clone();
        let captured = attempt
            .captured
            .as_ref()
            .ok_or(ServiceError::InvalidDocument)?;
        if !captured.is_dirty() {
            let outcome = DraftSaveOutcome::Unchanged {
                identity: attempt.identity,
                operation: operation.clone(),
            };
            let mut collection = state.drafts.clone();
            let stored = collection
                .attempts
                .get_mut(operation)
                .ok_or(ServiceError::InvalidDocument)?;
            stored.outcome = Some(outcome.clone());
            stored.captured = None;
            state.persist_draft_collection(&collection, state.blobs()?)?;
            state.drafts = collection;
            return Ok(stamped(session, outcome));
        }
        // The encrypted attempt survives a lost reply or uncertainty after publication.
        state.persist_draft_collection(&state.drafts, state.blobs()?)?;
        let mut candidate = state.document().clone();
        let receipt = taypeer_document::CommandReceipt {
            operation,
            kind: "save_draft_snapshot",
            fingerprint: &attempt.fingerprint,
        };
        let result = match captured {
            CapturedDraft::Entry(draft) => {
                if draft.adds_binary_content() {
                    let mut refs = state.document().blob_references()?.attachments;
                    refs.extend(
                        draft
                            .document()?
                            .fields()
                            .attachments
                            .values()
                            .map(|attachment| attachment.blob.clone()),
                    );
                    if state.blobs()?.unique_bytes(&refs) > state.policy().total_attachment_bytes()
                    {
                        return Err(ServiceError::AttachmentLimit);
                    }
                }
                serde_json::to_value(draft.save_command(&mut candidate, now, &receipt)?)
                    .map_err(|_| ServiceError::InvalidDocument)?
            }
            CapturedDraft::Metadata(draft) => draft.save_command(&mut candidate, now, &receipt)?,
        };
        state.commit(candidate)?;
        let document = state.document().clone();
        let outcome = state
            .drafts
            .finish_snapshot(operation, &document, &result)?;
        // The durable document receipt owns confirmation. A failed local rewrite is
        // reconciled from that receipt at reopen and never changes Saved into Unsaved.
        state.draft_maintenance_pending = state
            .persist_draft_collection(&state.drafts, state.blobs()?)
            .is_err();
        Ok(stamped(session, outcome))
    }

    /// Re-read authenticated durable state after uncertain publication without reauthenticating.
    /// Session, schema and author permissions remain enforced before accepting that state.
    pub fn reconcile_uncertain(
        &mut self,
        session: &SessionToken,
    ) -> Result<SessionValue<()>, ServiceError> {
        self.checked(session)?;
        let state = self
            .databases
            .get_mut(&session.database)
            .ok_or(ServiceError::NotFound)?;
        if !state.write_uncertain {
            return Ok(stamped(session, ()));
        }
        if let Some(managed) = &state.managed {
            managed.check_edit_permission()?;
        }
        let (document, mut blobs) = if let Some(managed) = &mut state.managed {
            managed.reload_confirmed()?
        } else {
            let (clear, blobs) = state
                .file
                .as_mut()
                .ok_or(ServiceError::InvalidContext)?
                .reload_bundle(state.key.as_ref().ok_or(ServiceError::Locked)?)?;
            (Document::load(&clear)?, blobs)
        };
        if document.database_id() != &session.database {
            return Err(ServiceError::InvalidContext);
        }
        managed::compatibility::require_write(
            &self.capabilities.assess(&document.schema_descriptor()?),
        )?;
        if document
            .blob_references()?
            .required
            .iter()
            .any(|id| blobs.length(id).is_none())
        {
            return Err(StorageError::MissingBlob.into());
        }
        // Draft blobs live independently of the published document candidate.
        if let Some(staged) = &state.blobs {
            blobs.import(&staged.retained(&state.drafts.binary_references()))?;
        }
        let mut collection = state.drafts.clone();
        collection.reconcile(&document)?;
        let document_changed = state.document().heads() != document.heads();
        if let Some(managed) = &mut state.managed {
            // Mark only after authenticated reload, compatibility, blob and draft validation.
            managed.reconciled_application |= document_changed;
        }
        state.drafts = collection;
        state.document = Some(document);
        state.blobs = Some(blobs);
        state.write_uncertain = false;
        Ok(stamped(session, ()))
    }

    /// Read group revisions and their retained original alternatives after authentication.
    pub fn group_history(
        &self,
        session: &SessionToken,
        group: &GroupId,
    ) -> Result<SessionValue<Vec<taypeer_core::SavedGroupRevision>>, ServiceError> {
        Ok(stamped(
            session,
            self.checked(session)?.document().group_history(group)?,
        ))
    }

    /// Read database name/description history after authentication.
    pub fn database_history(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Vec<taypeer_core::SavedDatabaseRevision>>, ServiceError> {
        Ok(stamped(
            session,
            self.checked(session)?.document().database_history()?,
        ))
    }

    /// Purge the selected group revisions atomically; the operation is retryable.
    pub fn purge_group_history(
        &mut self,
        session: &SessionToken,
        group: &GroupId,
        revisions: BTreeSet<RevisionId>,
        operation: &OperationId,
    ) -> Result<SessionValue<()>, ServiceError> {
        self.checked_mut(session)?
            .change(|document| Ok(document.purge_group_history(group, revisions, operation)?))?;
        Ok(stamped(session, ()))
    }

    /// Purge selected metadata revisions without reverting current descriptive fields.
    pub fn purge_database_history(
        &mut self,
        session: &SessionToken,
        revisions: BTreeSet<RevisionId>,
        operation: &OperationId,
    ) -> Result<SessionValue<()>, ServiceError> {
        self.checked_mut(session)?
            .change(|document| Ok(document.purge_database_history(revisions, operation)?))?;
        Ok(stamped(session, ()))
    }
}

fn apply_metadata_name(target: &mut String, update: FieldUpdate<String>) {
    match update {
        FieldUpdate::Keep => {}
        FieldUpdate::Set(value) => *target = value,
        FieldUpdate::Clear => target.clear(),
    }
}

fn apply_metadata_optional<T>(target: &mut Option<T>, update: FieldUpdate<T>) {
    match update {
        FieldUpdate::Keep => {}
        FieldUpdate::Set(value) => *target = Some(value),
        FieldUpdate::Clear => *target = None,
    }
}

#[cfg(test)]
mod tests;
