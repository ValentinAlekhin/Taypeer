use taypeer_services::EditableEntry;

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum EntryField {
    Title,
    Username,
    Password,
    Url,
    Notes,
    Tags,
    Expiry,
}
impl EntryField {
    pub(super) const ALL: [Self; 7] = [
        Self::Title,
        Self::Username,
        Self::Password,
        Self::Url,
        Self::Notes,
        Self::Tags,
        Self::Expiry,
    ];
    pub(super) fn key(self) -> &'static str {
        match self {
            Self::Title => "title",
            Self::Username => "username",
            Self::Password => "password",
            Self::Url => "url",
            Self::Notes => "notes",
            Self::Tags => "tags",
            Self::Expiry => "expires",
        }
    }
    pub(super) fn optional(self) -> bool {
        matches!(
            self,
            Self::Username | Self::Password | Self::Url | Self::Notes
        )
    }
    // Presentation conversion only; required names and domain invariants remain in services/core.
    pub(super) fn apply(self, fields: &mut EditableEntry, text: String) -> Result<(), ()> {
        match self {
            Self::Title => fields.title = text,
            Self::Username => fields.username = Some(text),
            Self::Password => fields.password = Some(text),
            Self::Url => fields.url = Some(text),
            Self::Notes => fields.notes = Some(text),
            Self::Tags => {
                fields.tags = if text.is_empty() {
                    Vec::new()
                } else {
                    text.split('\n').map(str::to_string).collect()
                }
            }
            Self::Expiry => {
                fields.expires_at = if text.is_empty() {
                    None
                } else {
                    Some(
                        chrono::NaiveDateTime::parse_from_str(&text, "%Y-%m-%d %H:%M")
                            .map_err(|_| ())?
                            .and_utc()
                            .timestamp_millis(),
                    )
                }
            }
        }
        Ok(())
    }
    pub(super) fn unset(self, fields: &mut EditableEntry) {
        match self {
            Self::Username => fields.username = None,
            Self::Password => fields.password = None,
            Self::Url => fields.url = None,
            Self::Notes => fields.notes = None,
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn optional_empty_and_absent_are_different_form_actions() {
        let mut fields = EditableEntry::default();
        EntryField::Username
            .apply(&mut fields, String::new())
            .unwrap();
        assert_eq!(fields.username, Some(String::new()));
        EntryField::Username.unset(&mut fields);
        assert_eq!(fields.username, None);
    }
    #[test]
    fn other_field_edits_preserve_exact_expiry_and_invalid_date_does_not_replace_it() {
        let mut fields = EditableEntry {
            expires_at: Some(1_800_000_000_123),
            ..Default::default()
        };
        EntryField::Title
            .apply(&mut fields, "Synthetic".into())
            .unwrap();
        assert!(
            EntryField::Expiry
                .apply(&mut fields, "2026-09-".into())
                .is_err()
        );
        assert_eq!(fields.expires_at, Some(1_800_000_000_123));
        EntryField::Expiry
            .apply(&mut fields, "1970-01-01 00:01".into())
            .unwrap();
        assert_eq!(fields.expires_at, Some(60_000));
    }
    #[test]
    fn tags_keep_unicode_whitespace_and_empty_lines() {
        let mut fields = EditableEntry::default();
        EntryField::Tags
            .apply(&mut fields, " Жук \n\nTag".into())
            .unwrap();
        assert_eq!(fields.tags, [" Жук ", "", "Tag"]);
    }
}
