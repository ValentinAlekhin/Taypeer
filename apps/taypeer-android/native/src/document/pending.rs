//! Masked late-source inspection and explicit extraction of permanently closed generations.
use super::{
    ChangedObject, DocumentSession, EntryRow, FormKind, GroupRow,
    session::{entry_row, identifier},
};
use crate::AndroidError;
use taypeer_core::{EntryField, FieldValue, GroupId, OperationId};
use taypeer_document::{ObjectId, PendingSource, RecoveryMode, RecoveryRequest};
use taypeer_runtime::Command;
use taypeer_services::{InspectionTarget, InspectionView};

/// Original content-event identity of a late change; no decrypted values are exported.
#[derive(uniffi::Record)]
pub struct PendingSourceRow {
    /// Stable event identity used by inspection and processing receipts.
    pub source: String,
    /// Entry or group; form-only categories are never returned.
    pub kind: FormKind,
    /// Original public identity whose closed lifetime stays closed after extraction.
    pub target: String,
    /// Exact permanently closed generation.
    pub generation: String,
    /// Original Automerge change hash, independent of ciphertext packaging.
    pub change: String,
    /// Original CRDT actor; this is not a network admission claim.
    pub actor: String,
    /// Original entry revision when present.
    pub revision: Option<String>,
}

/// One masked password alternative. The client must select its exact origins explicitly.
#[derive(uniffi::Record)]
pub struct PendingPasswordVariant {
    /// Immutable operations identifying this alternative, without its protected value.
    pub origins: Vec<String>,
}

/// Selected late source at its exact original state, with every password alternative masked.
#[derive(uniffi::Record)]
pub struct PendingInspection {
    /// Masked original entry, absent for a group source.
    pub entry: Option<EntryRow>,
    /// Original name, parent and icon, absent for an entry source.
    pub group: Option<GroupRow>,
    /// Exact password alternatives; none is implicitly chosen when several exist.
    pub password_variants: Vec<PendingPasswordVariant>,
}

fn pending_source(source: PendingSource) -> PendingSourceRow {
    let kind = match &source.address.object {
        ObjectId::Entry(_) => FormKind::Entry,
        ObjectId::Group(_) => FormKind::Group,
    };
    PendingSourceRow {
        source: source.id,
        kind,
        target: source.address.object.as_str().to_owned(),
        generation: source.address.generation.to_string(),
        change: source.change,
        actor: source.actor,
        revision: source.revision.map(|revision| revision.to_string()),
    }
}

#[uniffi::export]
impl DocumentSession {
    /// List unprocessed late changes of closed generations. These original sources
    /// are already authenticated document history, separate from unadmitted packets.
    pub fn pending_sources(&self) -> Result<Vec<PendingSourceRow>, AndroidError> {
        let sources: Vec<PendingSource> = self.command(Command::PendingSources)?;
        Ok(sources.into_iter().map(pending_source).collect())
    }

    /// Inspect one immutable late event without restoring its original generation.
    /// Source provenance remains inspectable after an explicit extraction.
    pub fn inspect_pending(&self, source: String) -> Result<PendingInspection, AndroidError> {
        let view: InspectionView = self.command(Command::Inspect(InspectionTarget::Source(
            identifier(source)?,
        )))?;
        Ok(match view {
            InspectionView::Entry { entry, fields, .. } => PendingInspection {
                entry: Some(entry_row(*entry)),
                group: None,
                password_variants: fields
                    .into_iter()
                    .filter(|field| field.field == EntryField::Password)
                    .flat_map(|field| field.variants)
                    .map(|variant| PendingPasswordVariant {
                        origins: variant.origins,
                    })
                    .collect(),
            },
            InspectionView::Group { group, .. } => PendingInspection {
                entry: None,
                group: Some(GroupRow {
                    id: group.address.object.as_str().to_owned(),
                    parent: group.placement.parent.map(|parent| parent.id.to_string()),
                    name: group.name,
                    // Shared source inspection currently projects group names, icons
                    // and placements without a descriptive-text field.
                    description: None,
                    icon: group.icon.into(),
                }),
                password_variants: Vec::new(),
            },
        })
    }

    /// Reveal only the explicitly chosen password origins from a late source.
    /// Common document semantics validate the exact variant; no alternative is inferred.
    pub fn reveal_pending(
        &self,
        source: String,
        origins: Vec<String>,
    ) -> Result<String, AndroidError> {
        let value: FieldValue = self.command(Command::RevealInspected {
            target: InspectionTarget::Source(identifier(source)?),
            field: EntryField::Password,
            origins: origins
                .into_iter()
                .map(identifier)
                .collect::<Result<_, _>>()?,
        })?;
        match value {
            FieldValue::Text(Some(value)) => Ok(value),
            FieldValue::Text(None) => Err(AndroidError::InvalidInput),
            _ => Err(AndroidError::Runtime),
        }
    }

    /// Clone exactly one late event into a new identity and current active destination.
    /// Keep source, destination and operation unchanged on an uncertain-result retry.
    /// Processing the event never reopens its old generation or grants network admission.
    pub fn recover_pending(
        &self,
        source: String,
        destination: Option<String>,
        operation: String,
    ) -> Result<ChangedObject, AndroidError> {
        let changed: Vec<ObjectId> = self.command(Command::RecoverSource {
            request: Box::new(RecoveryRequest {
                icon: None,
                source: identifier(source)?,
                mode: RecoveryMode::Clone,
                destination: destination.map(identifier).transpose()?.map(GroupId::new),
                name: None,
                fields: None,
            }),
            operation: OperationId::new(identifier(operation)?),
        })?;
        let [changed] = changed.as_slice() else {
            return Err(AndroidError::Runtime);
        };
        let kind = match changed {
            ObjectId::Entry(_) => FormKind::Entry,
            ObjectId::Group(_) => FormKind::Group,
        };
        Ok(ChangedObject {
            kind,
            target: changed.as_str().to_owned(),
        })
    }
}
