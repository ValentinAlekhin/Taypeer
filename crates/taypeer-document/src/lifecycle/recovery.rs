//! Late-source provenance and explicit recovery into a fresh lifetime.

use super::*;
use objects::MutationEvent;

/// Identity policy for extracting a retained late source.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecoveryMode {
    /// Preserve the public ID and start a fresh generation.
    Restore,
    /// Create a separate object with fresh public and attribute IDs.
    Clone,
}

/// An explicit extraction of one immutable source. Other sources remain pending.
#[derive(Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RecoveryRequest {
    /// Explicit group-icon choice when the original source has conflicting icons.
    #[serde(default)]
    pub icon: Option<IconRef>,
    /// Stable source event returned by the pending list.
    pub source: String,
    /// Identity policy.
    pub mode: RecoveryMode,
    /// Current destination; required for an entry, optional for a top-level group.
    pub destination: Option<GroupId>,
    /// Explicit group name choice; required if the source names conflict.
    pub name: Option<String>,
    /// Explicit full entry field choice; required if source fields conflict.
    pub fields: Option<EntryFields>,
}
impl std::fmt::Debug for RecoveryRequest {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("RecoveryRequest([REDACTED])")
    }
}

/// Original source state, before later merges. Callers must mask protected values.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum SourcePreview {
    /// Group names and placements at the source change.
    Group(GroupNode),
    /// Entry field variants at the source change.
    Entry(Box<EntrySnapshot>),
}
impl std::fmt::Debug for SourcePreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("SourcePreview([REDACTED])")
    }
}

#[derive(Serialize, Deserialize)]
struct RecoveryReceipt {
    operation: OperationId,
    source: PendingSource,
    target: ObjectAddress,
}

impl Document {
    /// Choose a current lifetime after concurrent recoveries, retaining unseen candidates.
    pub fn resolve_generation(
        &mut self,
        address: &ObjectAddress,
        heads: &[String],
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<Vec<ObjectId>, Error> {
        let intent = ("resolve_generation", address, heads);
        if let Some(result) = self.action_receipt(operation, &intent)? {
            return Ok(result);
        }
        let basis = self.at_heads(heads)?;
        if !objects::current(&basis.doc, &address.object)?.contains(address)
            || objects::purge(&self.doc, address)?.is_some()
        {
            return Err(Error::InvalidContext);
        }
        let hashes = parse_heads(&self.doc, heads)?;
        let mut candidate = self.clone();
        let mut tx = candidate.doc.transaction_at(PatchLog::null(), &hashes);
        let shell = objects::shell(&tx, &address.object)?;
        tx.put(shell, "current", address.generation.as_str())?;
        match address.object {
            ObjectId::Group(_) => objects::record_event(&mut tx, address, None)?,
            ObjectId::Entry(_) => confirm_revision(
                &mut tx,
                address,
                RevisionKind::Resolve,
                now,
                heads,
                RevisionId::new(random_id()),
            )?,
        }
        let result = vec![address.object.clone()];
        put_receipt(&mut tx, operation, &intent, &result)?;
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(result)
    }

    fn late_source(&self, id: &str) -> Result<PendingSource, Error> {
        let events = object(&self.doc, &ROOT, "events")?;
        let values = self.doc.get_all(events, id)?;
        let [(value, operation)] = values.as_slice() else {
            return Err(Error::NotFound);
        };
        let event: MutationEvent = decode(value)?;
        let purge = objects::purge(&self.doc, &event.address)?.ok_or(Error::NotFound)?;
        if purge.covered.contains(id) {
            return Err(Error::NotFound);
        }
        let hash = self
            .doc
            .hash_for_opid(operation)
            .ok_or(Error::InvalidDocument)?;
        let change = self
            .doc
            .get_change_by_hash(&hash)
            .ok_or(Error::InvalidDocument)?;
        Ok(PendingSource {
            id: id.to_owned(),
            address: event.address,
            change: hash.to_string(),
            actor: change.actor_id().to_hex_string(),
            revision: event.revision,
        })
    }

    /// List unprocessed late changes of closed generations, with original provenance.
    pub fn pending_sources(&self) -> Result<Vec<PendingSource>, Error> {
        let events = object(&self.doc, &ROOT, "events")?;
        let processed = object(&self.doc, &ROOT, "recoveries")?;
        let mut result = Vec::new();
        for id in self.doc.keys(&events) {
            let event: MutationEvent = decode(&unique(&self.doc, &events, &id)?)?;
            if objects::purge(&self.doc, &event.address)?.is_some_and(|p| !p.covered.contains(&id))
                && self.doc.get_all(&processed, &id)?.is_empty()
            {
                result.push(self.late_source(&id)?);
            }
        }
        Ok(result)
    }

    /// Inspect an original late source, including after processing for provenance review.
    /// This explicit API does not expose content already covered by a purge.
    pub fn preview_source(&self, id: &str) -> Result<SourcePreview, Error> {
        let source = self.late_source(id)?;
        let basis = self.at_heads(&[source.change])?;
        match source.address.object {
            ObjectId::Group(_) => Ok(SourcePreview::Group(groups::read_group(
                &basis.doc,
                &source.address,
            )?)),
            ObjectId::Entry(_) => Ok(SourcePreview::Entry(Box::new(
                projection::read_entry_generation(&basis.doc, &source.address, None)?,
            ))),
        }
    }

    /// Extract a source and its processing receipt atomically. Closed lifetimes stay closed.
    /// A retry returns its original result; reuse with another request is rejected.
    pub fn recover_source(
        &mut self,
        request: &RecoveryRequest,
        operation: &OperationId,
        now: Timestamp,
    ) -> Result<Vec<ObjectId>, Error> {
        let intent = ("recover_source", request);
        if let Some(result) = self.action_receipt(operation, &intent)? {
            return Ok(result);
        }
        let source = self.late_source(&request.source)?;
        let processed = object(&self.doc, &ROOT, "recoveries")?;
        if !self.doc.get_all(processed, &source.id)?.is_empty() {
            return Err(Error::InvalidContext);
        }
        let preview = self.preview_source(&source.id)?;
        let destination = self.destination(request.destination.clone())?;
        if matches!(source.address.object, ObjectId::Entry(_)) && destination.is_none() {
            return Err(Error::InvalidContext);
        }
        if request.mode == RecoveryMode::Restore {
            let current = objects::current(&self.doc, &source.address.object)?;
            for address in current {
                if objects::purge(&self.doc, &address)?.is_none() {
                    return Err(Error::Conflict);
                }
            }
        }
        let target = match request.mode {
            RecoveryMode::Restore => ObjectAddress {
                object: source.address.object.clone(),
                generation: GenerationId::new(random_id()),
            },
            RecoveryMode::Clone => {
                let id = match source.address.object {
                    ObjectId::Group(_) => ObjectId::Group(GroupId::new(random_id())),
                    ObjectId::Entry(_) => ObjectId::Entry(EntryId::new(random_id())),
                };
                ObjectAddress {
                    generation: GenerationId::new(id.as_str()),
                    object: id,
                }
            }
        };
        let order = self.position(
            destination.as_ref(),
            &SiblingPosition::Last,
            None,
            operation,
        )?;
        let heads = self.heads();
        let mut candidate = self.clone();
        let mut tx = candidate.doc.transaction();
        let node = match request.mode {
            RecoveryMode::Clone => objects::initialize(&mut tx, &target.object)?.1,
            RecoveryMode::Restore => {
                let shell = objects::shell(&tx, &target.object)?;
                let generations = object(&tx, &shell, "generations")?;
                let node = tx.put_object(generations, target.generation.as_str(), ObjType::Map)?;
                tx.put(shell, "current", target.generation.as_str())?;
                objects::put_life(&mut tx, &node, false, operation.clone(), BTreeSet::new())?;
                node
            }
        };
        match preview {
            SourcePreview::Group(original) => {
                if request.fields.is_some() {
                    return Err(Error::InvalidContext);
                }
                let name = match &request.name {
                    Some(name) => name.as_str(),
                    None => {
                        let [name] = original.names.as_slice() else {
                            return Err(Error::Conflict);
                        };
                        name
                    }
                };
                validate_group_name(name)?;
                let created = if request.mode == RecoveryMode::Restore {
                    original.created_at
                } else {
                    now
                };
                commands::initialize_group(
                    &mut tx,
                    &node,
                    name,
                    GroupPlacement {
                        parent: destination,
                        order,
                    },
                    now,
                )?;
                let icon = match &request.icon {
                    Some(icon) => icon,
                    None => {
                        let [icon] = original.icons.as_slice() else {
                            return Err(Error::Conflict);
                        };
                        icon
                    }
                };
                tx.put(&node, "icon", encode(icon)?)?;
                tx.put(&node, "created_at", created)?;
                objects::record_event(&mut tx, &target, None)?;
            }
            SourcePreview::Entry(original) => {
                if request.name.is_some() || request.icon.is_some() {
                    return Err(Error::InvalidContext);
                }
                let mut fields = request
                    .fields
                    .clone()
                    .or(original.fields)
                    .ok_or(Error::Conflict)?;
                if request.mode == RecoveryMode::Clone {
                    crate::binary::renew_attachment_ids(&mut fields);
                    fields.attributes = fields
                        .attributes
                        .into_values()
                        .map(|mut attr| {
                            attr.id = AttributeId::new(random_id());
                            (attr.id.clone(), attr)
                        })
                        .collect();
                }
                let created = if request.mode == RecoveryMode::Restore {
                    original.created_at
                } else {
                    now
                };
                let revision = RevisionId::new(random_id());
                commands::initialize_entry(
                    &mut tx,
                    &target,
                    &fields,
                    destination.as_ref().ok_or(Error::InvalidContext)?,
                    created,
                    &revision,
                )?;
                confirm_revision(
                    &mut tx,
                    &target,
                    if request.mode == RecoveryMode::Clone {
                        RevisionKind::Clone
                    } else {
                        RevisionKind::Restore
                    },
                    now,
                    &heads,
                    revision,
                )?;
            }
        }
        let result = vec![target.object.clone()];
        let receipts = object(&tx, &ROOT, "recoveries")?;
        tx.put(
            receipts,
            &source.id,
            encode(&RecoveryReceipt {
                operation: operation.clone(),
                source: source.clone(),
                target,
            })?,
        )?;
        put_receipt(&mut tx, operation, &intent, &result)?;
        tx.commit();
        candidate.validate_structure()?;
        *self = candidate;
        Ok(result)
    }
}
