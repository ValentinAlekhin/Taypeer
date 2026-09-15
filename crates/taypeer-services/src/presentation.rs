//! Session-checked projections and addressed forms shared by graphical clients.
use crate::*;
use serde::{Deserialize, Serialize};
use taypeer_core::{Appearance, Attachment, AttributeId, IconRef, OperationId};
use zeroize::Zeroize;

/// Descriptive metadata available only after authentication.
#[derive(Clone, Serialize, Deserialize)]
pub struct DatabaseInfo {
    /// Logical identity.
    pub id: DatabaseId,
    /// Current display name.
    pub name: String,
    /// Exact optional descriptive text.
    pub description: Option<String>,
    /// Concurrent descriptive alternatives require explicit resolution.
    pub metadata_conflict: bool,
    /// Whether the authenticated local author may write.
    pub writable: bool,
    /// Whether management settings may be changed.
    pub managing: bool,
}
/// A masked editor projection; secrets are requested separately.
#[derive(Clone, Serialize, Deserialize)]
pub struct EditorView {
    /// Existing target, absent for creation.
    pub entry: Option<EntryId>,
    /// Owning group.
    pub group: GroupId,
    /// Ordinary form values; password and protected attribute values are masked.
    pub fields: EditableEntry,
    /// Distinguishes an absent password from an explicitly empty password.
    pub has_password: bool,
    /// Whether the service has unconfirmed changes.
    pub dirty: bool,
    /// Exact invalid or unfinished expiration input.
    pub expiry_input: Option<String>,
    /// Shared draft attachment metadata.
    pub attachments: Vec<Attachment>,
    /// Shared draft appearance.
    pub appearance: Appearance,
    /// Verified attachment availability and sizes in this draft.
    pub binary: BinaryView,
    /// Unique local custom icon, if present and available.
    pub icon_preview: Option<IconPreview>,
}
/// Stable attribute editing without revealing unchanged protected values.
#[derive(Serialize, Deserialize)]
pub struct AttributePatch {
    /// Existing identity; absent only for creation.
    pub id: Option<AttributeId>,
    /// Exact name.
    pub name: String,
    /// Keep preserves the current secret even when its masked UI field is empty.
    pub value: FieldUpdate<String>,
    /// Atomic protection choice, combined with the retained or edited value.
    pub protected: bool,
}
/// A whole group form, committed once after validation.
#[derive(Serialize, Deserialize)]
pub struct GroupForm {
    /// Existing group or creation.
    pub id: Option<GroupId>,
    /// Parent for creation; editing retains its current placement.
    pub parent: Option<GroupId>,
    /// Exact name.
    pub name: String,
    /// Optional text.
    pub description: Option<String>,
    /// Local catalog or existing icon; acquisition uses the binary API.
    pub icon: IconRef,
}
/// Detailed group form values.
#[derive(Clone, Serialize, Deserialize)]
pub struct GroupInfo {
    /// Number of active direct entries, independent of the current UI search.
    pub entry_count: usize,
    /// Concurrent description alternatives require explicit resolution.
    pub description_conflict: bool,
    /// Ordinary group summary.
    pub group: GroupSummary,
    /// Optional descriptive text.
    pub description: Option<String>,
}
impl DatabaseService {
    /// Read current database metadata after session validation.
    pub fn database_info(&self, session: &SessionToken) -> Result<DatabaseInfo, ServiceError> {
        let doc = self.checked(session)?.document();
        Ok(DatabaseInfo {
            id: session.database.clone(),
            name: match doc.display_name() {
                Ok(name) => name,
                Err(taypeer_document::Error::Conflict) => doc.name().to_owned(),
                Err(error) => return Err(error.into()),
            },
            description: match doc.description() {
                Ok(value) => value,
                Err(taypeer_document::Error::Conflict) => None,
                Err(error) => return Err(error.into()),
            },
            metadata_conflict: matches!(doc.display_name(), Err(taypeer_document::Error::Conflict))
                || matches!(doc.description(), Err(taypeer_document::Error::Conflict)),
            writable: self.can_write(session)?,
            managing: self.can_manage(session)?,
        })
    }
    /// Confirm the complete metadata form atomically.
    pub fn update_database_info(
        &mut self,
        session: &SessionToken,
        name: String,
        description: Option<String>,
    ) -> Result<(), ServiceError> {
        self.checked_mut(session)?
            .change(|doc| Ok(doc.update_metadata(name, description)?))
    }
    /// Read descriptions without choosing conflicting alternatives.
    pub fn group_info(&self, session: &SessionToken) -> Result<Vec<GroupInfo>, ServiceError> {
        let doc = self.checked(session)?.document();
        let mut counts = std::collections::BTreeMap::<GroupId, usize>::new();
        for entry in self.entries(session, None, "")?.value {
            if let Some(group) = entry.group_id {
                *counts.entry(group).or_default() += 1;
            }
        }
        self.groups(session)?
            .value
            .into_iter()
            .map(|group| {
                Ok(GroupInfo {
                    entry_count: counts.get(&group.id).copied().unwrap_or(0),
                    description: match doc.group_description(&group.id) {
                        Ok(value) => value,
                        Err(taypeer_document::Error::Conflict) => None,
                        Err(error) => return Err(error.into()),
                    },
                    description_conflict: matches!(
                        doc.group_description(&group.id),
                        Err(taypeer_document::Error::Conflict)
                    ),
                    group,
                })
            })
            .collect()
    }
    /// Confirm name, description and icon as one durable group form.
    pub fn save_group_form(
        &mut self,
        session: &SessionToken,
        form: GroupForm,
        operation: &OperationId,
    ) -> Result<GroupId, ServiceError> {
        let now = (self.clock)();
        let intent = serde_json::to_value(&form).map_err(|_| ServiceError::InvalidInput)?;
        let state = self.checked_mut(session)?;
        if state.draft.is_some() {
            return Err(ServiceError::EditorAlreadyOpen);
        }
        state.change(|doc| {
            if let Some(id) = doc.binary_receipt(operation, &intent)? {
                return match id.as_slice() {
                    [ObjectId::Group(id)] => Ok(id.clone()),
                    _ => Err(ServiceError::InvalidContext),
                };
            }
            let id = match form.id {
                Some(id) => {
                    doc.rename_group(&id, form.name, now)?;
                    id
                }
                None => doc.create_group(form.name, form.parent, now)?.id,
            };
            doc.set_group_description(&id, form.description, now)?;
            doc.set_group_icon(&id, form.icon, None, operation, &intent, now)?;
            Ok(id)
        })
    }
    /// Project the active form with secret values erased before it crosses IPC.
    pub fn editor_view(&self, session: &SessionToken) -> Result<EditorView, ServiceError> {
        let state = self.checked(session)?;
        let draft = state.draft.as_ref().ok_or(ServiceError::NoDraft)?;
        let document = draft.document()?;
        let mut view = draft.view();
        let has_password = view.fields.password.is_some();
        view.fields.password.zeroize();
        view.fields.password = None;
        for attribute in &mut view.fields.attributes {
            if attribute.protected {
                attribute.value.zeroize();
            }
        }
        Ok(EditorView {
            entry: view.entry_id,
            group: view.group_id,
            fields: view.fields,
            has_password,
            dirty: view.dirty,
            expiry_input: view.expiry_input,
            attachments: document.fields().attachments.values().cloned().collect(),
            appearance: document.fields().appearance.clone(),
            binary: self.binary_view(session, &BinaryTarget::Draft)?.value,
            icon_preview: self.icon_preview(session, &BinaryTarget::Draft)?,
        })
    }
    /// Explicitly reveal the current active draft secret, never a saved stale version.
    pub fn reveal_editor(
        &self,
        session: &SessionToken,
        attribute: Option<&AttributeId>,
    ) -> Result<SecretValue, ServiceError> {
        let state = self.checked(session)?;
        let fields = state
            .draft
            .as_ref()
            .ok_or(ServiceError::NoDraft)?
            .document()?
            .fields();
        let value = match attribute {
            Some(id) => fields.attributes.get(id).map(|a| a.value.value.clone()),
            None => fields.password.clone(),
        }
        .ok_or(ServiceError::NotFound)?;
        Ok(SecretValue::new(value))
    }
    /// Edit/remove precisely one attribute, preserving all others and their identities.
    pub fn patch_attribute(
        &mut self,
        session: &SessionToken,
        patch: AttributePatch,
        remove: bool,
    ) -> Result<(), ServiceError> {
        let mut fields = self
            .draft(session)?
            .value
            .ok_or(ServiceError::NoDraft)?
            .fields;
        let existing = patch.id.as_ref().and_then(|id| {
            fields
                .attributes
                .iter()
                .position(|a| a.id.as_ref() == Some(id))
        });
        if patch.id.is_some() && existing.is_none() {
            return Err(ServiceError::NotFound);
        }
        if remove {
            fields
                .attributes
                .remove(existing.ok_or(ServiceError::NotFound)?);
        } else {
            let value = match patch.value {
                FieldUpdate::Keep => existing
                    .map(|i| fields.attributes[i].value.clone())
                    .unwrap_or_default(),
                FieldUpdate::Set(value) => value,
                FieldUpdate::Clear => String::new(),
            };
            let value = EditableAttribute {
                id: patch.id,
                name: patch.name,
                value,
                protected: patch.protected,
            };
            if let Some(index) = existing {
                fields.attributes[index] = value;
            } else {
                fields.attributes.push(value);
            }
        }
        taypeer_core::validate_attribute_names(fields.attributes.iter().map(|a| a.name.as_str()))
            .map_err(|_| ServiceError::InvalidInput)?;
        self.update_draft(session, fields)?;
        Ok(())
    }
}

/// Initial form applied before the first encrypted file is published.
#[derive(Clone, Serialize, Deserialize)]
pub struct CreateDatabase {
    /// Exact database label.
    pub name: String,
    /// Optional descriptive text.
    pub description: Option<String>,
    /// Initial shared protection and attachment limits.
    pub policy: taypeer_core::DatabasePolicy,
}
