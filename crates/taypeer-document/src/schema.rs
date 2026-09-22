//! Creation and decoding are separate: selecting a new writer must not remove a reader.
use super::*;

const VERSION: u16 = 5;

pub(super) fn initialize(
    doc: &mut Automerge,
    database_id: &DatabaseId,
    name: &str,
    now: Timestamp,
) -> Result<(), Error> {
    // Adding a writer requires an explicit initializer rather than relabeling v5 bytes.
    match taypeer_core::CURRENT_SCHEMA {
        5 => initialize_v5(doc, database_id, name, now),
        _ => Err(Error::UnsupportedSchema),
    }
}
fn initialize_v5(
    doc: &mut Automerge,
    database_id: &DatabaseId,
    name: &str,
    now: Timestamp,
) -> Result<(), Error> {
    let mut tx = doc.transaction();
    tx.put(ROOT, "schema", u64::from(VERSION))?;
    tx.put(
        ROOT,
        "schema_descriptor",
        codec::encode(&taypeer_core::SchemaDescriptor::current())?,
    )?;
    tx.put(ROOT, "database_id", database_id.as_str())?;
    tx.put(ROOT, "name", name)?;
    tx.put(ROOT, "created_at", now)?;
    tx.put_object(ROOT, "groups", ObjType::Map)?;
    tx.put_object(ROOT, "entries", ObjType::Map)?;
    tx.put_object(ROOT, "revisions", ObjType::Map)?;
    tx.put_object(ROOT, "operations", ObjType::Map)?;
    tx.put_object(ROOT, "purged_revisions", ObjType::Map)?;
    for root in ["events", "purges", "lifecycle_receipts", "recoveries"] {
        tx.put_object(ROOT, root, ObjType::Map)?;
    }
    tx.commit();
    Ok(())
}

pub(super) fn load(bytes: &[u8]) -> Result<Document, Error> {
    let doc = Automerge::load(bytes)?;
    let schema = unique(&doc, &ROOT, "schema")?
        .to_u64()
        .filter(|value| *value > 0 && *value <= u64::from(u16::MAX))
        .ok_or(Error::InvalidDocument)?;
    match schema {
        5 => load_v5(doc),
        _ => Err(Error::UnsupportedSchema),
    }
}

fn load_v5(doc: Automerge) -> Result<Document, Error> {
    let database_id = DatabaseId::new(
        unique(&doc, &ROOT, "database_id")?
            .to_str()
            .ok_or(Error::InvalidDocument)?,
    );
    let name = unique(&doc, &ROOT, "name")?
        .to_str()
        .ok_or(Error::InvalidDocument)?
        .to_owned();
    validate_group_name(&name)?;
    let document = Document {
        database_id,
        name,
        doc,
        writer: None,
    };
    document.validate_structure()?;
    Ok(document)
}

impl Document {
    pub(super) fn validate_structure(&self) -> Result<(), Error> {
        let descriptor = self.schema_descriptor()?;
        if !taypeer_core::ClientCapabilities::default()
            .assess(&descriptor)
            .read
            .is_supported()
        {
            return Err(Error::UnsupportedSchema);
        }
        match descriptor.schema_version() {
            5 => self.validate_v5_structure(),
            _ => Err(Error::UnsupportedSchema),
        }
    }
}
