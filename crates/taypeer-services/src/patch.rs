//! Addressed editor updates shared by command-line and graphical clients.

use crate::{DatabaseService, EditableAttribute, ServiceError, SessionToken, SessionValue};
use serde::{Deserialize, Serialize};
use zeroize::Zeroize;

/// An explicit update; an omitted property preserves the current form value.
#[derive(Default, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FieldUpdate<T> {
    /// Keep the current value, including the difference between empty and absent.
    #[default]
    Keep,
    /// Store the exact supplied value.
    Set(T),
    /// Remove the optional value or empty a collection.
    Clear,
}

impl<T> std::fmt::Debug for FieldUpdate<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Keep => "Keep",
            Self::Set(_) => "Set([REDACTED])",
            Self::Clear => "Clear",
        })
    }
}

/// Addressed changes to the active draft; omitted fields are never overwritten.
#[derive(Default, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct EntryPatch {
    /// Required title; clearing is rejected.
    pub title: FieldUpdate<String>,
    /// Optional login.
    pub username: FieldUpdate<String>,
    /// Optional password, used without normalization.
    pub password: FieldUpdate<String>,
    /// Optional URL; setting it does not access the network.
    pub url: FieldUpdate<String>,
    /// Optional atomic notes.
    pub notes: FieldUpdate<String>,
    /// Atomic tag set.
    pub tags: FieldUpdate<Vec<String>>,
    /// Optional UTC timestamp in milliseconds.
    pub expires_at: FieldUpdate<i64>,
    /// Attribute collection, preserving existing identities.
    pub attributes: FieldUpdate<Vec<EditableAttribute>>,
}

impl EntryPatch {
    fn apply(mut self, fields: &mut crate::EditableEntry) -> Result<(), ServiceError> {
        match std::mem::take(&mut self.title) {
            FieldUpdate::Keep => {}
            FieldUpdate::Set(title) => fields.title = title,
            FieldUpdate::Clear => return Err(ServiceError::InvalidInput),
        }
        optional(&mut fields.username, std::mem::take(&mut self.username));
        optional(&mut fields.password, std::mem::take(&mut self.password));
        optional(&mut fields.url, std::mem::take(&mut self.url));
        optional(&mut fields.notes, std::mem::take(&mut self.notes));
        optional(&mut fields.expires_at, std::mem::take(&mut self.expires_at));
        collection(&mut fields.tags, std::mem::take(&mut self.tags));
        collection(&mut fields.attributes, std::mem::take(&mut self.attributes));
        Ok(())
    }
}

impl EntryPatch {
    /// Erase this owned input buffer after dispatch; this does not erase other copies.
    pub fn erase(&mut self) {
        for update in [
            &mut self.title,
            &mut self.username,
            &mut self.password,
            &mut self.url,
            &mut self.notes,
        ] {
            if let FieldUpdate::Set(value) = update {
                value.zeroize();
            }
        }
        if let FieldUpdate::Set(tags) = &mut self.tags {
            tags.zeroize();
        }
        if let FieldUpdate::Set(attributes) = &mut self.attributes {
            for attribute in attributes {
                attribute.name.zeroize();
                attribute.value.zeroize();
            }
        }
    }
}

fn optional<T>(target: &mut Option<T>, update: FieldUpdate<T>) {
    match update {
        FieldUpdate::Keep => {}
        FieldUpdate::Set(value) => *target = Some(value),
        FieldUpdate::Clear => *target = None,
    }
}

fn collection<T>(target: &mut Vec<T>, update: FieldUpdate<Vec<T>>) {
    match update {
        FieldUpdate::Keep => {}
        FieldUpdate::Set(value) => *target = value,
        FieldUpdate::Clear => target.clear(),
    }
}

impl DatabaseService {
    /// Apply only submitted fields to the active local draft. Errors preserve the form.
    /// The response contains no editor contents; confirmation is a separate command.
    pub fn patch_draft(
        &mut self,
        session: &SessionToken,
        patch: EntryPatch,
    ) -> Result<SessionValue<()>, ServiceError> {
        let mut fields = self
            .draft(session)?
            .value
            .ok_or(ServiceError::NoDraft)?
            .fields;
        patch.apply(&mut fields)?;
        self.update_draft(session, fields)?;
        Ok(super::stamped(session, ()))
    }

    /// Close a database, revoking access and releasing its writer lock even on draft failure.
    pub fn close_database(&mut self, database: &crate::DatabaseId) -> Result<(), ServiceError> {
        let mut state = self
            .databases
            .remove(database)
            .ok_or(ServiceError::NotFound)?;
        if state.unlocked {
            state.stash_and_close()?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{DEMO_PASSWORD, EditableEntry};

    #[test]
    fn patch_preserves_omissions_and_distinguishes_empty_from_clear() {
        let mut service = DatabaseService::new();
        let db = service.create_database("PUBLIC patch").unwrap();
        let session = service.unlock(&db, DEMO_PASSWORD).unwrap();
        let group = service
            .create_group(&session, "PUBLIC group".into(), None)
            .unwrap()
            .value;
        service.start_create_entry(&session, group.id).unwrap();
        service
            .update_draft(
                &session,
                EditableEntry {
                    title: "PUBLIC entry".into(),
                    password: Some("PUBLIC_KEEP_ME".into()),
                    username: Some("PUBLIC login".into()),
                    ..Default::default()
                },
            )
            .unwrap();
        let patch = serde_json::from_str(
            r#"{"username":{"action":"set","value":""},"notes":{"action":"clear"}}"#,
        )
        .unwrap();
        service.patch_draft(&session, patch).unwrap();
        let fields = service.draft(&session).unwrap().value.unwrap().fields;
        assert_eq!(fields.username.as_deref(), Some(""));
        assert_eq!(fields.password.as_deref(), Some("PUBLIC_KEEP_ME"));
        assert_eq!(fields.notes, None);
        assert!(
            service
                .patch_draft(
                    &session,
                    EntryPatch {
                        title: FieldUpdate::Clear,
                        ..Default::default()
                    }
                )
                .is_err()
        );
        assert_eq!(
            service.draft(&session).unwrap().value.unwrap().fields,
            fields
        );
    }
}
