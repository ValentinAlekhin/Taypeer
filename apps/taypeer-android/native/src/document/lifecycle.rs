//! Typed retained objects and session-owned exact lifecycle review capabilities.
use super::{DocumentSession, FormKind, failure, session::identifier};
use crate::AndroidError;
use std::collections::BTreeMap;
use taypeer_core::{EntryId, GenerationId, GroupId, OperationId};
use taypeer_runtime::Command;
use taypeer_services::{
    InspectionTarget, InspectionView, LifecycleAction, ObjectId, ObjectState, PreparedLifecycle,
};

const MAX_REVIEWS: usize = 64;

/// Masked retained metadata, addressed to the exact displayed generation.
#[derive(uniffi::Record)]
pub struct TrashRow {
    /// Entry or group; editor-only categories are never returned.
    pub kind: FormKind,
    /// Stable object identity.
    pub target: String,
    /// Retained generation, checked before preparing a review.
    pub generation: String,
    /// Selected ordinary name; no protected field crosses this projection.
    pub name: String,
    /// Selected original group/parent, absent for a root group.
    pub group: Option<String>,
}
/// Composition of the exact reviewed selection. The causal selection stays in Rust.
#[derive(uniffi::Record)]
pub struct LifecycleReview {
    /// Opaque capability usable only through the same live DocumentSession.
    pub review: String,
    /// Number of reviewed groups.
    pub groups: u32,
    /// Number of reviewed entries.
    pub entries: u32,
}
/// IDs created or affected by a shared subtree operation.
#[derive(uniffi::Record)]
pub struct ChangedObject {
    /// Entry or group.
    pub kind: FormKind,
    /// Stable returned object identity.
    pub target: String,
}

#[derive(Default)]
pub(super) struct LifecycleReviews {
    next: u64,
    selections: BTreeMap<String, ReviewedSelection>,
}
struct ReviewedSelection {
    prepared: PreparedLifecycle,
    operation: Option<OperationId>,
}
impl LifecycleReviews {
    fn insert(
        &mut self,
        generation: u64,
        prepared: PreparedLifecycle,
    ) -> Result<LifecycleReview, AndroidError> {
        if self.selections.len() >= MAX_REVIEWS {
            return Err(AndroidError::InvalidOptions);
        }
        self.next = self.next.checked_add(1).ok_or(AndroidError::Runtime)?;
        let identity = serde_json::to_vec(&(generation, self.next, &prepared)).map_err(failure)?;
        let review = taypeer_trust::Digest::of(&identity).to_string();
        let groups = prepared
            .affected
            .keys()
            .filter(|address| matches!(address.object, ObjectId::Group(_)))
            .count();
        let entries = prepared.affected.len() - groups;
        let view = LifecycleReview {
            review: review.clone(),
            groups: groups.try_into().map_err(failure)?,
            entries: entries.try_into().map_err(failure)?,
        };
        self.selections.insert(
            review,
            ReviewedSelection {
                prepared,
                operation: None,
            },
        );
        Ok(view)
    }
}
fn object(kind: FormKind, target: String) -> Result<ObjectId, AndroidError> {
    Ok(match kind {
        FormKind::Entry => ObjectId::Entry(EntryId::new(identifier(target)?)),
        FormKind::Group => ObjectId::Group(GroupId::new(identifier(target)?)),
        FormKind::NewEntry | FormKind::NewGroup | FormKind::Database => {
            return Err(AndroidError::InvalidOptions);
        }
    })
}
fn changed(object: ObjectId) -> ChangedObject {
    match object {
        ObjectId::Entry(id) => ChangedObject {
            kind: FormKind::Entry,
            target: id.to_string(),
        },
        ObjectId::Group(id) => ChangedObject {
            kind: FormKind::Group,
            target: id.to_string(),
        },
    }
}
fn validate_reviewed_generation(
    prepared: &PreparedLifecycle,
    target: &ObjectId,
    generation: &GenerationId,
) -> Result<(), AndroidError> {
    let mut roots = prepared
        .affected
        .keys()
        .filter(|address| &address.object == target);
    if matches!((roots.next(), roots.next()), (Some(root), None) if &root.generation == generation)
    {
        Ok(())
    } else {
        Err(AndroidError::InvalidOptions)
    }
}
impl DocumentSession {
    fn prepare_trash_review(
        &self,
        action: LifecycleAction,
        kind: FormKind,
        target: String,
        generation: String,
        destination: Option<String>,
    ) -> Result<LifecycleReview, AndroidError> {
        let target = object(kind, target)?;
        let generation = GenerationId::new(identifier(generation)?);
        let prepared: PreparedLifecycle = self.command(Command::PrepareLifecycle {
            action,
            target: target.clone(),
            destination: destination.map(identifier).transpose()?.map(GroupId::new),
        })?;
        validate_reviewed_generation(&prepared, &target, &generation)?;
        let (database, session) = self.identity()?;
        if prepared.database.as_str() != database {
            return Err(AndroidError::Runtime);
        }
        self.lifecycle_reviews
            .lock()
            .map_err(failure)?
            .insert(session, prepared)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn a_stale_trash_row_cannot_review_a_recovered_generation_of_the_same_entry() {
        let mut document = taypeer_document::Document::new("PUBLIC generation review", 1).unwrap();
        let group = document
            .create_group("PUBLIC retained group".into(), None, 2)
            .unwrap()
            .id;
        let mut draft = document.begin_create_entry(group.clone()).unwrap();
        draft.fields_mut().title = "PUBLIC retained entry".into();
        let entry = document.save_entry(draft, 3).unwrap();
        let target = ObjectId::Entry(entry.clone());
        let old_generation = document.entry(&entry).unwrap().generation;
        let mut late = document.fork();
        let mut draft = late.begin_edit_entry(&entry).unwrap();
        draft.fields_mut().title = "PUBLIC late retained title".into();
        late.save_entry(draft, 4).unwrap();
        document
            .trash_object(
                target.clone(),
                &OperationId::new("PUBLIC old generation trash"),
                5,
            )
            .unwrap();
        let purge = document
            .prepare_lifecycle(LifecycleAction::Purge, target.clone(), None)
            .unwrap();
        document
            .confirm_lifecycle(&purge, &OperationId::new("PUBLIC old generation purge"), 6)
            .unwrap();
        document.merge(&late).unwrap();
        let source = document.pending_sources().unwrap().pop().unwrap();
        document
            .recover_source(
                &taypeer_document::RecoveryRequest {
                    icon: None,
                    source: source.id,
                    mode: taypeer_document::RecoveryMode::Restore,
                    destination: Some(group),
                    name: None,
                    fields: None,
                },
                &OperationId::new("PUBLIC recovered generation"),
                7,
            )
            .unwrap();
        let recovered_generation = document.entry(&entry).unwrap().generation;
        assert_ne!(old_generation, recovered_generation);
        document
            .trash_object(
                target.clone(),
                &OperationId::new("PUBLIC recovered generation trash"),
                8,
            )
            .unwrap();
        let prepared = document
            .prepare_lifecycle(LifecycleAction::Purge, target.clone(), None)
            .unwrap();
        let before_review = document.export();
        assert!(matches!(
            validate_reviewed_generation(&prepared, &target, &old_generation),
            Err(AndroidError::InvalidOptions)
        ));
        assert_eq!(document.export(), before_review);
        validate_reviewed_generation(&prepared, &target, &recovered_generation).unwrap();
        document
            .confirm_lifecycle(
                &prepared,
                &OperationId::new("PUBLIC explicit recovered generation purge"),
                9,
            )
            .unwrap();
        assert!(document.entry(&entry).is_err());
    }
}
#[uniffi::export]
impl DocumentSession {
    /// List retained trash names with all protected values masked in the Rust service.
    pub fn trash_rows(&self) -> Result<Vec<TrashRow>, AndroidError> {
        let objects: Vec<ObjectState> = self.command(Command::Trash)?;
        objects
            .into_iter()
            .map(|state| {
                let target = state.address.object.as_str().to_owned();
                let generation = state.address.generation.to_string();
                let view: InspectionView =
                    self.command(Command::Inspect(InspectionTarget::Object(state.address)))?;
                Ok(match view {
                    InspectionView::Group { group, .. } => TrashRow {
                        kind: FormKind::Group,
                        target,
                        generation,
                        name: group.name,
                        group: group.placement.parent.map(|parent| parent.id.to_string()),
                    },
                    InspectionView::Entry { entry, .. } => TrashRow {
                        kind: FormKind::Entry,
                        target,
                        generation,
                        name: entry.title,
                        group: Some(entry.group_id.to_string()),
                    },
                })
            })
            .collect()
    }
    /// Capture an exact restoration selection before displaying confirmation.
    /// Entries require an active group; root restoration is supported for groups.
    pub fn prepare_trash_restore(
        &self,
        kind: FormKind,
        target: String,
        generation: String,
        destination: Option<String>,
    ) -> Result<LifecycleReview, AndroidError> {
        self.prepare_trash_review(
            LifecycleAction::Restore,
            kind,
            target,
            generation,
            destination,
        )
    }
    /// Capture exact retained generations before a permanent purge confirmation.
    pub fn prepare_trash_purge(
        &self,
        kind: FormKind,
        target: String,
        generation: String,
    ) -> Result<LifecycleReview, AndroidError> {
        self.prepare_trash_review(LifecycleAction::Purge, kind, target, generation, None)
    }
    /// Confirm the stored selection without recomputing it. Keep the capability
    /// and operation for uncertain-result retries; discard it only after the UI
    /// has handled durable success or cancellation.
    pub fn confirm_trash_review(
        &self,
        review: String,
        operation: String,
    ) -> Result<(), AndroidError> {
        self.identity()?;
        let operation = OperationId::new(identifier(operation)?);
        let prepared = {
            let mut reviews = self.lifecycle_reviews.lock().map_err(failure)?;
            let selection = reviews
                .selections
                .get_mut(&identifier(review)?)
                .ok_or(AndroidError::InvalidOptions)?;
            if selection
                .operation
                .as_ref()
                .is_some_and(|previous| previous != &operation)
            {
                return Err(AndroidError::InvalidOptions);
            }
            selection.operation = Some(operation.clone());
            selection.prepared.clone()
        };
        self.command::<Vec<ObjectId>>(Command::ConfirmLifecycle {
            prepared,
            operation,
        })
        .map(|_| ())
    }
    /// Release one unused or completed review. This never changes database data.
    pub fn discard_trash_review(&self, review: String) -> Result<(), AndroidError> {
        self.lifecycle_reviews
            .lock()
            .map_err(failure)?
            .selections
            .remove(&identifier(review)?);
        Ok(())
    }
    /// Clone one active entry with new identities through the shared durable command.
    pub fn clone_entry(
        &self,
        entry: String,
        group: String,
        title: Option<String>,
        operation: String,
    ) -> Result<String, AndroidError> {
        if title
            .as_ref()
            .is_some_and(|value| value.len() > super::MAX_METADATA)
        {
            return Err(AndroidError::InvalidOptions);
        }
        let id: EntryId = self.command(Command::CloneEntry {
            entry: EntryId::new(identifier(entry)?),
            group: GroupId::new(identifier(group)?),
            title,
            operation: OperationId::new(identifier(operation)?),
        })?;
        Ok(id.to_string())
    }
    /// Clone an active subtree and return the new group/entry IDs. Shared rules
    /// validate placement, names and the exact operation's durable retry receipt.
    pub fn clone_group(
        &self,
        group: String,
        parent: Option<String>,
        name: Option<String>,
        operation: String,
    ) -> Result<Vec<ChangedObject>, AndroidError> {
        if name
            .as_ref()
            .is_some_and(|value| value.len() > super::MAX_METADATA)
        {
            return Err(AndroidError::InvalidOptions);
        }
        self.command::<Vec<ObjectId>>(Command::CloneGroup {
            group: GroupId::new(identifier(group)?),
            parent: parent.map(identifier).transpose()?.map(GroupId::new),
            name,
            operation: OperationId::new(identifier(operation)?),
        })
        .map(|objects| objects.into_iter().map(changed).collect())
    }
    /// Move an entry to the explicitly selected active group; validation and
    /// receipt semantics remain in the common service.
    pub fn move_entry(
        &self,
        entry: String,
        group: String,
        operation: String,
    ) -> Result<(), AndroidError> {
        self.command::<Vec<ObjectId>>(Command::MoveEntry {
            entry: EntryId::new(identifier(entry)?),
            group: GroupId::new(identifier(group)?),
            review: None,
            operation: OperationId::new(identifier(operation)?),
        })
        .map(|_| ())
    }
    /// Move an active group to the last sibling position of its selected parent,
    /// or the root. The shared service rejects cycles and inactive destinations.
    pub fn move_group(
        &self,
        group: String,
        parent: Option<String>,
        operation: String,
    ) -> Result<(), AndroidError> {
        self.command::<Vec<ObjectId>>(Command::MoveGroup {
            request: taypeer_services::GroupMove {
                group: GroupId::new(identifier(group)?),
                parent: parent.map(identifier).transpose()?.map(GroupId::new),
                position: taypeer_services::SiblingPosition::Last,
                review: None,
                name: None,
            },
            operation: OperationId::new(identifier(operation)?),
        })
        .map(|_| ())
    }
}
