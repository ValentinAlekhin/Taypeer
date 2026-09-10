//! Session gates, masked inspection and durable publication of lifecycle commands.

use super::*;
use serde::{Deserialize, Serialize};
use taypeer_core::{EntryField, FieldValue, GroupRef, OperationId};
use taypeer_document::SourcePreview;

/// Lossless tree and availability projection with its causal review context.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct TreeView {
    /// Heads to submit with explicit conflict choices.
    pub heads: Vec<String>,
    /// Retained groups, including all placement/name alternatives.
    pub groups: Vec<GroupNode>,
    /// Retained object availability, including entries without a usable destination.
    pub objects: Vec<ObjectState>,
}

/// Explicit scope for inspecting retained data outside ordinary entry workflows.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum InspectionTarget {
    /// Exact retained generation; a purge prevents access.
    Object(ObjectAddress),
    /// Immutable late source event, before subsequent edits.
    Source(String),
}

/// Protected data is absent from both the unique view and field alternatives.
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum InspectionView {
    /// Retained group metadata.
    Group {
        /// Causal review context.
        heads: Vec<String>,
        /// Every name and placement alternative.
        group: GroupNode,
    },
    /// Retained entry metadata and masked alternatives.
    Entry {
        /// Causal review context.
        heads: Vec<String>,
        /// Exact lifetime.
        address: ObjectAddress,
        /// Unique fields when available, with protected values removed.
        entry: Box<EntryView>,
        /// Atomic destination alternatives.
        placements: Vec<GroupRef>,
        /// Field alternatives, including origins for explicit reveal.
        fields: Vec<ConflictFieldView>,
    },
}

pub(super) fn inspect(
    document: &Document,
    target: &InspectionTarget,
) -> Result<SourcePreview, ServiceError> {
    Ok(match target {
        InspectionTarget::Source(id) => document.preview_source(id)?,
        InspectionTarget::Object(address) => match &address.object {
            ObjectId::Group(_) => SourcePreview::Group(
                document
                    .tree()?
                    .into_iter()
                    .find(|n| &n.address == address)
                    .ok_or(ServiceError::NotFound)?,
            ),
            ObjectId::Entry(_) => SourcePreview::Entry(Box::new(document.inspect_entry(address)?)),
        },
    })
}

impl DatabaseService {
    /// List retained trash objects independently of the active group tree.
    pub fn trash(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Vec<ObjectState>>, ServiceError> {
        let states = self.checked(session)?.document().object_states()?;
        Ok(stamped(
            session,
            states
                .into_iter()
                .filter(|s| s.status == ObjectStatus::Trashed)
                .collect(),
        ))
    }

    /// Read the acyclic ordinary tree alongside retained placement/lifecycle conflicts.
    pub fn tree(&self, session: &SessionToken) -> Result<SessionValue<TreeView>, ServiceError> {
        let document = self.checked(session)?.document();
        Ok(stamped(
            session,
            TreeView {
                heads: document.review_heads(),
                groups: document.tree()?,
                objects: document.object_states()?,
            },
        ))
    }

    /// Inspect retained metadata without exposing passwords or protected attributes.
    pub fn inspect_object(
        &self,
        session: &SessionToken,
        target: &InspectionTarget,
    ) -> Result<SessionValue<InspectionView>, ServiceError> {
        let document = self.checked(session)?.document();
        let heads = document.review_heads();
        let view = match inspect(document, target)? {
            SourcePreview::Group(group) => InspectionView::Group { heads, group },
            SourcePreview::Entry(snapshot) => InspectionView::Entry {
                heads,
                address: ObjectAddress {
                    object: ObjectId::Entry(snapshot.id.clone()),
                    generation: snapshot.generation.clone(),
                },
                placements: snapshot.placements.clone(),
                fields: operations::mask_fields(snapshot.values.clone()),
                entry: Box::new(entry_view(*snapshot)),
            },
        };
        Ok(stamped(session, view))
    }

    /// Reveal precisely one inspected field variant using its immutable origins.
    pub fn reveal_inspected(
        &self,
        session: &SessionToken,
        target: &InspectionTarget,
        field: &EntryField,
        origins: &[String],
    ) -> Result<SessionValue<FieldValue>, ServiceError> {
        let SourcePreview::Entry(snapshot) = inspect(self.checked(session)?.document(), target)?
        else {
            return Err(ServiceError::InvalidInput);
        };
        let value = snapshot
            .values
            .into_iter()
            .find(|s| &s.field == field)
            .and_then(|s| s.variants.into_iter().find(|v| v.origins == origins))
            .ok_or(ServiceError::NotFound)?
            .value;
        Ok(stamped(session, value))
    }

    /// Read original provenance for unprocessed changes of closed generations.
    pub fn pending_sources(
        &self,
        session: &SessionToken,
    ) -> Result<SessionValue<Vec<PendingSource>>, ServiceError> {
        Ok(stamped(
            session,
            self.checked(session)?.document().pending_sources()?,
        ))
    }

    /// Prepare an exact reviewable selection without changing the database.
    pub fn prepare_lifecycle(
        &self,
        session: &SessionToken,
        action: LifecycleAction,
        target: ObjectId,
        destination: Option<GroupId>,
    ) -> Result<SessionValue<PreparedLifecycle>, ServiceError> {
        Ok(stamped(
            session,
            self.checked(session)?
                .document()
                .prepare_lifecycle(action, target, destination)?,
        ))
    }

    fn lifecycle_change(
        &mut self,
        session: &SessionToken,
        change: impl FnOnce(&mut Document, i64) -> Result<Vec<ObjectId>, taypeer_document::Error>,
    ) -> Result<SessionValue<Vec<ObjectId>>, ServiceError> {
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(editor_open_error(state));
        }
        let result = state.change(|doc| Ok(change(doc, now)?))?;
        Ok(stamped(session, result))
    }

    /// Confirm the exact reviewed selection and receipt only after durable storage.
    pub fn confirm_lifecycle(
        &mut self,
        session: &SessionToken,
        prepared: &PreparedLifecycle,
        operation: &OperationId,
    ) -> Result<SessionValue<Vec<ObjectId>>, ServiceError> {
        self.lifecycle_change(session, |doc, now| {
            doc.confirm_lifecycle(prepared, operation, now)
        })
    }

    /// Move a group or resolve its reviewed placement/name alternatives.
    pub fn move_group(
        &mut self,
        session: &SessionToken,
        request: &GroupMove,
        operation: &OperationId,
    ) -> Result<SessionValue<Vec<ObjectId>>, ServiceError> {
        self.lifecycle_change(session, |doc, now| doc.move_group(request, operation, now))
    }

    /// Move an entry, optionally using reviewed heads to resolve placement alternatives.
    pub fn move_entry(
        &mut self,
        session: &SessionToken,
        entry: &EntryId,
        group: GroupId,
        review: Option<Vec<String>>,
        operation: &OperationId,
    ) -> Result<SessionValue<Vec<ObjectId>>, ServiceError> {
        self.lifecycle_change(session, |doc, now| {
            doc.move_entry(entry, group, review, operation, now)
        })
    }

    /// Clone a complete active subtree with new identities and new entry histories.
    pub fn clone_group(
        &mut self,
        session: &SessionToken,
        group: &GroupId,
        parent: Option<GroupId>,
        name: Option<String>,
        operation: &OperationId,
    ) -> Result<SessionValue<Vec<ObjectId>>, ServiceError> {
        self.lifecycle_change(session, |doc, now| {
            doc.clone_group(group, parent, name, operation, now)
        })
    }

    /// Recover a late source and save its processing receipt atomically.
    pub fn recover_source(
        &mut self,
        session: &SessionToken,
        request: &RecoveryRequest,
        operation: &OperationId,
    ) -> Result<SessionValue<Vec<ObjectId>>, ServiceError> {
        self.lifecycle_change(session, |doc, now| {
            doc.recover_source(request, operation, now)
        })
    }

    /// Select an explicitly reviewed lifetime after concurrent recoveries.
    pub fn resolve_generation(
        &mut self,
        session: &SessionToken,
        address: &ObjectAddress,
        heads: &[String],
        operation: &OperationId,
    ) -> Result<SessionValue<Vec<ObjectId>>, ServiceError> {
        self.lifecycle_change(session, |doc, now| {
            doc.resolve_generation(address, heads, operation, now)
        })
    }
}
