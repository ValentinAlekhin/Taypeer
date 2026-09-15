//! Optional descriptive metadata, updated without reserializing unknown document fields.
use super::*;

impl Document {
    /// Read a unique optional description; concurrent alternatives require review.
    pub fn description(&self) -> Result<Option<String>, Error> {
        optional_text(&self.doc, &ROOT, "description")
    }
    /// Current display name; the original required name remains readable by older clients.
    pub fn display_name(&self) -> Result<String, Error> {
        Ok(optional_text(&self.doc, &ROOT, "display_name")?.unwrap_or_else(|| self.name.clone()))
    }
    /// Update independent descriptive fields, preserving all unrelated and unknown values.
    pub fn update_metadata(
        &mut self,
        name: String,
        description: Option<String>,
    ) -> Result<(), Error> {
        validate_group_name(&name)?;
        let name_changed = self.display_name()? != name;
        let description_changed = self.description()? != description;
        if !name_changed && !description_changed {
            return Ok(());
        }
        self.prepare_write()?;
        let mut tx = self.doc.transaction();
        if name_changed {
            tx.put(ROOT, "display_name", name)?;
        }
        if description_changed {
            tx.put(ROOT, "description", encode(&description)?)?;
        }
        tx.commit();
        Ok(())
    }
    /// Optional group description, kept on its precise state generation.
    pub fn group_description(&self, id: &GroupId) -> Result<Option<String>, Error> {
        self.require_group(id)?;
        let address = objects::single(&self.doc, &objects::ObjectId::Group(id.clone()))?;
        let object = objects::generation_object(&self.doc, &address)?;
        optional_text(&self.doc, &object, "description")
    }
    /// Change a group's descriptive text without changing its identity or placement.
    pub fn set_group_description(
        &mut self,
        id: &GroupId,
        description: Option<String>,
        now: Timestamp,
    ) -> Result<(), Error> {
        if self.group_description(id)? == description {
            return Ok(());
        }
        self.prepare_write()?;
        let address = objects::single(&self.doc, &objects::ObjectId::Group(id.clone()))?;
        let mut tx = self.doc.transaction();
        let object = objects::generation_object(&tx, &address)?;
        tx.put(&object, "description", encode(&description)?)?;
        tx.put(&object, "description_modified_at", now)?;
        objects::record_event(&mut tx, &address, None)?;
        tx.commit();
        Ok(())
    }
}
pub(super) fn optional_text(
    read: &impl ReadDoc,
    object: &automerge::ObjId,
    key: &str,
) -> Result<Option<String>, Error> {
    let mut variants = Vec::new();
    for (value, _) in read.get_all(object, key)? {
        let text = if key == "display_name" {
            Some(value.to_str().ok_or(Error::InvalidDocument)?.to_owned())
        } else {
            codec::decode::<Option<String>>(&value)?
        };
        if !variants.contains(&text) {
            variants.push(text);
        }
    }
    if variants.len() > 1 {
        return Err(Error::Conflict);
    }
    Ok(variants.pop().flatten())
}

#[cfg(test)]
mod tests {
    use super::*;
    use taypeer_core::OperationId;
    #[test]
    fn optional_metadata_roundtrips_and_survives_ordinary_edits() {
        let mut document = Document::new("PUBLIC original", 1000).unwrap();
        let group = document
            .create_group("PUBLIC group".into(), None, 1000)
            .unwrap()
            .id;
        assert_eq!(document.description().unwrap(), None);
        assert_eq!(document.group_description(&group).unwrap(), None);
        let mut tx = document.doc.transaction();
        tx.put(ROOT, "future_optional", "PUBLIC unknown preserved")
            .unwrap();
        tx.commit();
        document
            .update_metadata(
                "PUBLIC renamed".into(),
                Some("  PUBLIC description\nТекст  ".into()),
            )
            .unwrap();
        document
            .set_group_description(&group, Some("PUBLIC group description".into()), 2000)
            .unwrap();
        let mut reloaded = Document::load(&document.export()).unwrap();
        reloaded
            .rename_group(&group, "PUBLIC changed group".into(), 3000)
            .unwrap();
        assert_eq!(reloaded.name(), "PUBLIC original");
        assert_eq!(reloaded.display_name().unwrap(), "PUBLIC renamed");
        assert_eq!(
            reloaded.description().unwrap().as_deref(),
            Some("  PUBLIC description\nТекст  ")
        );
        assert_eq!(
            reloaded.group_description(&group).unwrap().as_deref(),
            Some("PUBLIC group description")
        );
        assert_eq!(
            reloaded
                .doc
                .get(ROOT, "future_optional")
                .unwrap()
                .unwrap()
                .0
                .to_str(),
            Some("PUBLIC unknown preserved")
        );
    }
    #[test]
    fn conflicting_metadata_is_not_silently_selected_or_overwritten() {
        let original = Document::new("PUBLIC original", 1000).unwrap();
        let mut left = original.fork();
        let mut right = original.fork();
        left.update_metadata("PUBLIC left".into(), None).unwrap();
        right.update_metadata("PUBLIC right".into(), None).unwrap();
        left.merge(&right).unwrap();
        assert_eq!(left.display_name(), Err(Error::Conflict));
        let before = left.export();
        assert_eq!(
            left.update_metadata("PUBLIC third".into(), None),
            Err(Error::Conflict)
        );
        assert_eq!(left.export(), before);
    }
    #[test]
    fn independent_metadata_edits_merge_without_rewriting_untouched_fields() {
        let original = Document::new("PUBLIC original", 1000).unwrap();
        let mut left = original.fork();
        let mut right = original.fork();
        left.update_metadata("PUBLIC renamed".into(), None).unwrap();
        right
            .update_metadata("PUBLIC original".into(), Some(String::new()))
            .unwrap();
        left.merge(&right).unwrap();
        assert_eq!(left.display_name().unwrap(), "PUBLIC renamed");
        assert_eq!(left.description().unwrap(), Some(String::new()));
    }
    #[test]
    fn cloned_group_keeps_its_description_and_identity_is_independent() {
        let mut document = Document::new("PUBLIC original", 1000).unwrap();
        let group = document
            .create_group("PUBLIC group".into(), None, 1000)
            .unwrap()
            .id;
        document
            .set_group_description(&group, Some("PUBLIC original description".into()), 2000)
            .unwrap();
        let objects = document
            .clone_group(
                &group,
                None,
                Some("PUBLIC clone".into()),
                &OperationId::new("PUBLIC clone"),
                3000,
            )
            .unwrap();
        let ObjectId::Group(clone) = &objects[0] else {
            panic!("expected group clone")
        };
        assert_ne!(clone, &group);
        document.set_group_description(&group, None, 4000).unwrap();
        let reloaded = Document::load(&document.export()).unwrap();
        assert_eq!(
            reloaded.group_description(clone).unwrap().as_deref(),
            Some("PUBLIC original description")
        );
        assert_eq!(reloaded.group_description(&group).unwrap(), None);
    }
}
