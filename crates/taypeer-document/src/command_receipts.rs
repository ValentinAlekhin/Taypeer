//! Exact command receipts share the existing encrypted operation namespace.
use crate::{
    Document, Error,
    codec::{decode, encode, object, unique_optional},
};
use automerge::{
    ROOT, ReadDoc,
    transaction::{Transactable, Transaction},
};
use serde::{Deserialize, Serialize};
use taypeer_core::OperationId;

#[derive(Serialize, Deserialize)]
struct Receipt {
    intent: serde_json::Value,
    result: serde_json::Value,
}
impl Document {
    /// Read a command result, rejecting IDs occupied by another operation kind.
    pub fn command_receipt(
        &self,
        operation: &OperationId,
        kind: &str,
    ) -> Result<Option<(serde_json::Value, serde_json::Value)>, Error> {
        if operation.as_str().is_empty() {
            return Err(Error::InvalidContext);
        }
        let old = object(&self.doc, &ROOT, "operations")?;
        if !self.doc.get_all(old, operation.as_str())?.is_empty() {
            return Err(Error::DuplicateId);
        }
        let root = object(&self.doc, &ROOT, "lifecycle_receipts")?;
        let Some(value) = unique_optional(&self.doc, &root, operation.as_str())? else {
            return Ok(None);
        };
        let receipt: Receipt = decode(&value)?;
        if receipt.intent.get("command").and_then(|v| v.as_str()) != Some(kind) {
            return Err(Error::DuplicateId);
        }
        Ok(Some((
            receipt.intent["fingerprint"].clone(),
            receipt.result,
        )))
    }
}

/// Borrowed exact-request identity prepared by the service before a document mutation.
pub struct CommandReceipt<'a> {
    /// Caller-retained operation identity.
    pub operation: &'a OperationId,
    /// Stable command discriminator.
    pub kind: &'a str,
    /// Digest only; never the plaintext request.
    pub fingerprint: &'a serde_json::Value,
}
impl CommandReceipt<'_> {
    pub(super) fn write(
        &self,
        tx: &mut Transaction<'_>,
        result: &impl Serialize,
    ) -> Result<(), Error> {
        if self.operation.as_str().is_empty() {
            return Err(Error::InvalidContext);
        }
        let old = object(tx, &ROOT, "operations")?;
        let root = object(tx, &ROOT, "lifecycle_receipts")?;
        if !tx.get_all(old, self.operation.as_str())?.is_empty()
            || !tx.get_all(&root, self.operation.as_str())?.is_empty()
        {
            return Err(Error::DuplicateId);
        }
        tx.put(
            root,
            self.operation.as_str(),
            encode(&Receipt {
                intent: serde_json::json!({"command":self.kind,"fingerprint":self.fingerprint}),
                result: serde_json::to_value(result).map_err(|_| Error::InvalidDocument)?,
            })?,
        )?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{LifecycleAction, ObjectId};
    #[test]
    fn command_receipts_share_the_history_binary_and_trash_namespace() {
        let mut doc = Document::new("PUBLIC receipt namespace", 1).unwrap();
        let group = doc.create_group("PUBLIC group".into(), None, 2).unwrap();
        let mut draft = doc.begin_create_entry(group.id.clone()).unwrap();
        draft.fields_mut().title = "PUBLIC entry".into();
        let entry = doc.save_entry(draft, 3).unwrap();
        let history = OperationId::new("PUBLIC history");
        doc.clone_entry(&entry, group.id.clone(), None, &history, 4)
            .unwrap();
        let binary = OperationId::new("PUBLIC binary");
        doc.record_binary_operation(&binary, &serde_json::json!({"fixture":true}))
            .unwrap();
        let trash = OperationId::new("PUBLIC trash");
        let prepared = doc
            .prepare_lifecycle(LifecycleAction::Trash, ObjectId::Entry(entry), None)
            .unwrap();
        doc.confirm_lifecycle(&prepared, &trash, 5).unwrap();
        let bytes = doc.export();
        for operation in [&history, &binary, &trash] {
            assert_eq!(
                doc.command_receipt(operation, "create_group"),
                Err(Error::DuplicateId)
            );
        }
        assert_eq!(doc.export(), bytes);
        let operation = OperationId::new("PUBLIC ordinary command");
        let fingerprint = serde_json::json!("PUBLIC digest fixture");
        let receipt = CommandReceipt {
            operation: &operation,
            kind: "create_group",
            fingerprint: &fingerprint,
        };
        let heads = doc.heads();
        doc.create_group_command("PUBLIC second".into(), None, 6, Some(&receipt))
            .unwrap();
        assert_eq!(doc.changes_since(&heads).unwrap().len(), 1);
        assert_eq!(
            doc.binary_receipt(&operation, &serde_json::json!({})),
            Err(Error::DuplicateId)
        );
        assert_eq!(
            doc.confirm_lifecycle(&prepared, &operation, 7),
            Err(Error::DuplicateId)
        );
    }
}
