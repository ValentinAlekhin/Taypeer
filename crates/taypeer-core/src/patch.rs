//! Addressed updates preserve the difference between untouched, supplied and absent values.

use crate::IconRef;
use serde::{Deserialize, Serialize};

/// An explicit update; an omitted property preserves the current value.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(
    tag = "action",
    content = "value",
    rename_all = "snake_case",
    deny_unknown_fields
)]
pub enum FieldUpdate<T> {
    /// Preserve the value, including the difference between empty and absent.
    #[default]
    Keep,
    /// Store the exact supplied value.
    Set(T),
    /// Remove an optional value or reset an explicitly clearable value.
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

/// Dirty descriptive fields of one group. Clearing its required name is invalid.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct GroupMetadataPatch {
    /// Exact group name.
    pub name: FieldUpdate<String>,
    /// Optional descriptive text.
    pub description: FieldUpdate<String>,
    /// Explicit icon; clearing restores the default icon.
    pub icon: FieldUpdate<IconRef>,
}

/// Dirty descriptive fields of the database. Clearing its required name is invalid.
#[derive(Clone, Default, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct DatabaseMetadataPatch {
    /// Exact display name.
    pub name: FieldUpdate<String>,
    /// Optional descriptive text.
    pub description: FieldUpdate<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn addressed_update_wire_shapes_keep_empty_distinct_from_absent() {
        let keep: FieldUpdate<String> = serde_json::from_str(r#"{"action":"keep"}"#).unwrap();
        let clear: FieldUpdate<String> = serde_json::from_str(r#"{"action":"clear"}"#).unwrap();
        let empty: FieldUpdate<String> =
            serde_json::from_str(r#"{"action":"set","value":""}"#).unwrap();
        assert_eq!(keep, FieldUpdate::Keep);
        assert_eq!(clear, FieldUpdate::Clear);
        assert_eq!(empty, FieldUpdate::Set(String::new()));
        assert_eq!(
            serde_json::to_string(&empty).unwrap(),
            r#"{"action":"set","value":""}"#
        );
        assert!(
            serde_json::from_str::<FieldUpdate<String>>(r#"{"action":"keep","extra":true}"#)
                .is_err()
        );
    }

    #[test]
    fn patch_diagnostics_redact_all_supplied_text() {
        let patch = DatabaseMetadataPatch {
            name: FieldUpdate::Set("PUBLIC diagnostic marker".into()),
            description: FieldUpdate::Set("PUBLIC diagnostic marker".into()),
        };
        assert!(!format!("{patch:?}").contains("marker"));
    }
}
