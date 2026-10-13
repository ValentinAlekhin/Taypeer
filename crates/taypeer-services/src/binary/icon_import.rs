//! Receipt-first acquisition into a causal form; this never saves a document revision.
use super::*;
use taypeer_core::DraftId;

/// Only an uncertain encrypted publication retains this exact candidate. The
/// exposed form stays unchanged until a retry confirms the same bytes durably.
pub(crate) struct PendingIconPublication {
    operation: OperationId,
    intent: IconReceiptIntent,
    identity: DraftIdentity,
    collection: DraftCollection,
    blobs: BlobStore,
}

impl PendingIconPublication {
    pub(crate) fn stash(&self, state: &DatabaseState) -> Result<(), ServiceError> {
        let mut collection = self.collection.clone();
        collection.stash();
        state.persist_draft_collection(&collection, &self.blobs)
    }
}

impl DatabaseState {
    fn publish_icon(
        &mut self,
        pending: PendingIconPublication,
    ) -> Result<DraftIdentity, ServiceError> {
        if let Err(error) = self.persist_draft_collection(&pending.collection, &pending.blobs) {
            if error == ServiceError::Storage(StorageError::CommitUncertain) {
                self.pending_icon = Some(pending);
            }
            return Err(error);
        }
        self.drafts = pending.collection;
        self.blobs = Some(pending.blobs);
        Ok(pending.identity)
    }
}

impl DatabaseService {
    /// Check the exact frozen image request before acquiring any selected input or
    /// network source. A receipt returns the current form identity without rolling
    /// back later input. Unacknowledged imports require that form to remain active.
    pub fn icon_import_receipt(
        &mut self,
        session: &SessionToken,
        draft_id: &DraftId,
        source: &IconSource,
        operation: &OperationId,
    ) -> Result<SessionValue<Option<DraftIdentity>>, ServiceError> {
        let intent = import_intent(draft_id, source, operation)?;
        let request = IconReceiptIntent::Import(source.clone());
        if let Some(identity) = self.confirm_pending_icon(session, draft_id, &request, operation)? {
            return Ok(stamped(session, Some(identity)));
        }
        let state = self.checked(session)?;
        if let Some(identity) = state.drafts.form_icon_receipt(
            draft_id,
            &IconReceiptIntent::Import(source.clone()),
            operation,
        )? {
            return Ok(stamped(session, Some(identity)));
        }
        let draft = state.drafts.captured(draft_id)?;
        require_icon_form(&draft)?;
        if draft.binary_receipt(operation, &intent)? {
            return Ok(stamped(session, Some(draft.identity())));
        }
        check_edit_permission(state, &self.capabilities)?;
        require_active_form(&state.drafts, draft_id)?;
        state.drafts.require_icon_capacity()?;
        Ok(stamped(session, None))
    }

    /// Validate the complete original image and durably stage it with the exact
    /// entry/group form. Acknowledged retries precede every read; the operation
    /// freezes the source choice, unlike content-bound attachment replacement.
    /// Invalid, short, long or failed streams leave the previous form/blob state
    /// intact. Encrypted local durability is separate from the common explicit Save.
    pub fn import_icon_stream(
        &mut self,
        session: &SessionToken,
        draft_id: &DraftId,
        mut input: impl Read,
        import: IconImport,
        operation: &OperationId,
    ) -> Result<SessionValue<DraftIdentity>, ServiceError> {
        if let Some(identity) = self
            .icon_import_receipt(session, draft_id, &import.source, operation)?
            .value
        {
            return Ok(stamped(session, identity));
        }
        if import.length > ICON_LIMIT {
            return Err(ServiceError::Icon(icons::IconError::TooLarge));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        input
            .by_ref()
            .take(import.length + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ServiceError::Icon(icons::IconError::File))?;
        if bytes.len() as u64 != import.length {
            return Err(ServiceError::InvalidInput);
        }
        icons::validate(&bytes).map_err(ServiceError::Icon)?;
        let intent = import_intent(draft_id, &import.source, operation)?;
        let state = self.checked_mut(session)?;
        let mut draft = state.drafts.captured(draft_id)?;
        let mut blobs = state.blobs()?.clone();
        let blob = blobs.insert(bytes.as_slice(), import.length, ICON_LIMIT)?;
        let source = import.source;
        let icon = IconRef::Image {
            blob,
            source: source.clone(),
        };
        draft.stage_icon(icon.clone(), operation.clone(), intent)?;
        let identity = draft.identity();
        let mut collection = state.drafts.clone();
        collection.replace_binary_form(draft)?;
        collection.record_icon_operation(
            identity.clone(),
            IconReceiptIntent::Import(source.clone()),
            icon,
            operation.clone(),
        )?;
        let identity = state.publish_icon(PendingIconPublication {
            operation: operation.clone(),
            intent: IconReceiptIntent::Import(source),
            identity,
            collection,
            blobs,
        })?;
        Ok(stamped(session, identity))
    }

    /// Offer one already-held favicon without acquiring any image or website.
    /// Current provenance wins, then the latest acknowledged original for this
    /// form, then its confirmed baseline. Missing bytes never choose another icon.
    pub fn stored_favicon(
        &self,
        session: &SessionToken,
        draft_id: &DraftId,
    ) -> Result<SessionValue<Option<StoredFavicon>>, ServiceError> {
        let state = self.checked(session)?;
        let blobs = state.blobs()?;
        let original = state
            .drafts
            .stored_favicon_icon(draft_id)?
            .and_then(|icon| icon.blob().cloned())
            .map(|identity| StoredFavicon {
                bytes: blobs.length(&identity),
                identity,
            });
        Ok(stamped(session, original))
    }

    /// Select the exact previously offered favicon original into the common form.
    /// A changed candidate, unavailable blob or stale form refuses without mutation.
    /// Receipt replay precedes candidate lookup and preserves later input/selections.
    pub fn select_stored_favicon(
        &mut self,
        session: &SessionToken,
        draft_id: &DraftId,
        original: &BlobId,
        operation: &OperationId,
    ) -> Result<SessionValue<DraftIdentity>, ServiceError> {
        if operation.as_str().is_empty() {
            return Err(ServiceError::InvalidInput);
        }
        let request = IconReceiptIntent::StoredFavicon(original.clone());
        if let Some(identity) = self.confirm_pending_icon(session, draft_id, &request, operation)? {
            return Ok(stamped(session, identity));
        }
        let state = self.checked(session)?;
        if let Some(identity) = state
            .drafts
            .form_icon_receipt(draft_id, &request, operation)?
        {
            return Ok(stamped(session, identity));
        }
        let intent = serde_json::json!({"action": "select_stored_favicon", "draft": draft_id, "identity": original});
        let mut draft = state.drafts.captured(draft_id)?;
        if draft.binary_receipt(operation, &intent)? {
            return Ok(stamped(session, draft.identity()));
        }
        let state = self.checked_mut(session)?;
        require_active_form(&state.drafts, draft_id)?;
        state.drafts.require_icon_capacity()?;
        let icon = state
            .drafts
            .stored_favicon_icon(draft_id)?
            .filter(|icon| icon.blob() == Some(original))
            .ok_or(ServiceError::InvalidInput)?;
        if state.blobs()?.length(original).is_none() {
            return Err(StorageError::MissingBlob.into());
        }
        draft.stage_icon(icon.clone(), operation.clone(), intent)?;
        let identity = draft.identity();
        let mut collection = state.drafts.clone();
        collection.replace_binary_form(draft)?;
        collection.record_icon_operation(
            identity.clone(),
            request.clone(),
            icon,
            operation.clone(),
        )?;
        let blobs = state.blobs()?.clone();
        let identity = state.publish_icon(PendingIconPublication {
            operation: operation.clone(),
            intent: request,
            identity,
            collection,
            blobs,
        })?;
        Ok(stamped(session, identity))
    }

    fn confirm_pending_icon(
        &mut self,
        session: &SessionToken,
        draft: &DraftId,
        intent: &IconReceiptIntent,
        operation: &OperationId,
    ) -> Result<Option<DraftIdentity>, ServiceError> {
        self.checked(session)?;
        let state = self
            .databases
            .get_mut(&session.database)
            .ok_or(ServiceError::NotFound)?;
        let Some(pending) = &state.pending_icon else {
            return Ok(None);
        };
        if &pending.operation != operation
            || &pending.identity.draft != draft
            || &pending.intent != intent
        {
            return Err(StorageError::CommitUncertain.into());
        }
        check_confirmed_edit_permission(state, &self.capabilities)?;
        state.persist_draft_collection(&pending.collection, &pending.blobs)?;
        let pending = state
            .pending_icon
            .take()
            .ok_or(ServiceError::InvalidContext)?;
        state.drafts = pending.collection;
        state.blobs = Some(pending.blobs);
        Ok(Some(pending.identity))
    }
}

fn import_intent(
    draft: &DraftId,
    source: &IconSource,
    operation: &OperationId,
) -> Result<serde_json::Value, ServiceError> {
    if operation.as_str().is_empty()
        || matches!(source, IconSource::Url(url) | IconSource::Favicon(url) if url.is_empty() || url.len() > 65536)
    {
        return Err(ServiceError::InvalidInput);
    }
    Ok(serde_json::json!({"action": "import_icon_stream", "draft": draft, "source": source}))
}

fn require_icon_form(draft: &CapturedDraft) -> Result<(), ServiceError> {
    if matches!(
        draft.identity().target,
        DraftTarget::Entry(_)
            | DraftTarget::NewEntry { .. }
            | DraftTarget::Group(_)
            | DraftTarget::NewGroup { .. }
    ) {
        Ok(())
    } else {
        Err(ServiceError::InvalidContext)
    }
}
