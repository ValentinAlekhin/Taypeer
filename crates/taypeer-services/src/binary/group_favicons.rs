//! Shared group selection and receipt-first saved-entry streaming; no HTTP in this gateway.
use super::*;
use std::collections::BTreeSet;

fn batch_intent(request: &GroupFaviconRequest) -> Result<serde_json::Value, ServiceError> {
    if request.operation.as_str().is_empty() {
        return Err(ServiceError::InvalidInput);
    }
    Ok(serde_json::json!({"platform_group_favicons": request}))
}

fn child_operation(batch: &OperationId, entry: &EntryId) -> OperationId {
    OperationId::new(format!("{}:favicon:{}", batch.as_str(), entry.as_str()))
}

fn entry_intent(request: &FaviconEntryRequest) -> Result<serde_json::Value, ServiceError> {
    batch_intent(&request.batch)?;
    if request.operation != child_operation(&request.batch.operation, &request.entry)
        || request.url.is_empty()
    {
        return Err(ServiceError::InvalidInput);
    }
    Ok(serde_json::json!({"platform_group_favicon_entry": request}))
}

impl DatabaseService {
    /// Select active saved entries in Rust and durably bind the parent operation.
    /// No network, editor activation or user history occurs. The returned rows
    /// belong to one bounded live batch; restarting does not reconstruct them.
    pub fn prepare_group_favicons(
        &mut self,
        session: &SessionToken,
        request: GroupFaviconRequest,
    ) -> Result<SessionValue<GroupFaviconPlan>, ServiceError> {
        self.reconcile_uncertain(session)?;
        let intent = batch_intent(&request)?;
        let state = self.checked_mut(session)?;
        // Validate even an old parent before reading mutable group membership.
        let acknowledged = state
            .document()
            .binary_receipt(&request.operation, &intent)?;
        let groups = state.document().groups()?;
        if !groups.iter().any(|group| group.id == request.group) {
            return Err(ServiceError::NotFound);
        }
        let mut selected = BTreeSet::from([request.group.clone()]);
        if request.recursive {
            loop {
                let count = selected.len();
                for group in &groups {
                    if group
                        .parent
                        .as_ref()
                        .is_some_and(|id| selected.contains(id))
                    {
                        selected.insert(group.id.clone());
                    }
                }
                if count == selected.len() {
                    break;
                }
            }
        }
        let mut rows = Vec::new();
        for entry in state.document().entries()? {
            if !selected.contains(&entry.group_id) {
                continue;
            }
            let fields = entry.fields.as_ref().ok_or(ServiceError::InvalidDocument)?;
            let action = if !request.replace && fields.appearance.icon != IconRef::Default {
                FaviconPlanAction::Skipped
            } else if let Some(url) = fields.url.as_ref().filter(|url| !url.is_empty()) {
                FaviconPlanAction::Acquire(FaviconEntryRequest {
                    batch: request.clone(),
                    entry: entry.id.clone(),
                    url: url.clone(),
                    operation: child_operation(&request.operation, &entry.id),
                })
            } else {
                FaviconPlanAction::MissingUrl
            };
            rows.push(FaviconPlanRow {
                entry: entry.id,
                title: fields.title.clone(),
                action,
            });
        }
        let plan = GroupFaviconPlan { request, rows };
        // Check before publishing the parent marker; an oversized UI response
        // must not become a partially prepared attempt.
        if serde_json::to_vec(&plan)
            .map_err(|_| ServiceError::InvalidDocument)?
            .len()
            > FAVICON_PLAN_LIMIT
        {
            return Err(ServiceError::InvalidInput);
        }
        if acknowledged.is_none() {
            let mut candidate = state.document().clone();
            candidate.record_binary_operation(&plan.request.operation, &intent)?;
            state.commit(candidate)?;
        }
        Ok(stamped(session, plan))
    }

    /// Reconcile unknown publication and inspect the exact child receipt before
    /// provider/HTTP access. An ACK survives later text, icon and group changes.
    pub fn favicon_entry_receipt(
        &mut self,
        session: &SessionToken,
        request: &FaviconEntryRequest,
    ) -> Result<SessionValue<bool>, ServiceError> {
        let intent = entry_intent(request)?;
        self.reconcile_uncertain(session)?;
        let state = self.checked(session)?;
        if state
            .document()
            .binary_receipt(&request.batch.operation, &batch_intent(&request.batch)?)?
            .is_none()
        {
            return Err(ServiceError::InvalidContext);
        }
        if state
            .document()
            .binary_receipt(&request.operation, &intent)?
            .is_some()
        {
            return Ok(stamped(session, true));
        }
        check_edit_permission(state, &self.capabilities)?;
        state.document().entry(&request.entry)?;
        Ok(stamped(session, false))
    }

    /// Confirm a bounded original favicon into a saved entry, independently of
    /// an unfinished local form. Exact receipt replay happens before input reads.
    /// Validated icon/blob/history/receipt share one existing durable commit.
    pub fn import_entry_favicon_stream(
        &mut self,
        session: &SessionToken,
        request: &FaviconEntryRequest,
        mut input: impl Read,
        length: u64,
    ) -> Result<SessionValue<()>, ServiceError> {
        if self.favicon_entry_receipt(session, request)?.value {
            return Ok(stamped(session, ()));
        }
        if length > ICON_LIMIT {
            return Err(ServiceError::Icon(icons::IconError::TooLarge));
        }
        let mut bytes = Zeroizing::new(Vec::new());
        input
            .by_ref()
            .take(length + 1)
            .read_to_end(&mut bytes)
            .map_err(|_| ServiceError::Icon(icons::IconError::File))?;
        if bytes.len() as u64 != length {
            return Err(ServiceError::InvalidInput);
        }
        icons::validate(&bytes).map_err(ServiceError::Icon)?;
        let intent = entry_intent(request)?;
        let now = (self.clock)();
        let state = self.checked_mut(session)?;
        let mut blobs = state.blobs()?.clone();
        let blob = blobs.insert(bytes.as_slice(), length, ICON_LIMIT)?;
        let mut draft = state.document().begin_edit_entry(&request.entry)?;
        draft.fields_mut().appearance.icon = IconRef::Image {
            blob,
            source: IconSource::Favicon(request.url.clone()),
        };
        let mut candidate = state.document().clone();
        candidate.save_binary_entry(draft, &request.operation, &intent, now)?;
        state.commit_blobs(candidate, blobs)?;
        Ok(stamped(session, ()))
    }
}

#[cfg(test)]
mod tests;
