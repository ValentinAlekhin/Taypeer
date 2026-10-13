//! Eligibility and value are checked in one sequential worker command, never in host metadata.
use taypeer_core::{AttributeId, EntryId, RevisionId};
use taypeer_services::{DatabaseService, SecretValue, ServiceError, SessionToken};

pub(super) fn reveal(
    service: &DatabaseService,
    session: &SessionToken,
    entry: &EntryId,
    revision: Option<&RevisionId>,
    attribute: Option<&AttributeId>,
) -> Result<SecretValue, ServiceError> {
    // Saved views and their secret reads share the same worker command. An incoming
    // application cannot change protected eligibility between these two steps.
    let view = match revision {
        Some(revision) => service.revision(session, entry, revision)?.value,
        None => service.view_entry(session, entry)?.value,
    };
    if let Some(attribute) = attribute
        && !view
            .attributes
            .iter()
            .any(|value| &value.id == attribute && value.protected)
    {
        return Err(ServiceError::InvalidInput);
    }
    Ok(match (revision, attribute) {
        (Some(revision), Some(attribute)) => {
            service
                .reveal_revision_attribute(session, entry, revision, attribute)?
                .value
        }
        (Some(revision), None) => {
            service
                .reveal_revision_password(session, entry, revision)?
                .value
        }
        (None, Some(attribute)) => service.reveal_attribute(session, entry, attribute)?.value,
        (None, None) => service.reveal_password(session, entry)?.value,
    })
}
