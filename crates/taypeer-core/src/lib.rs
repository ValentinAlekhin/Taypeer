//! Platform-independent values for the first, deliberately volatile Taypeer database.
//!
//! These types define no file format, authentication, or durable storage contract.

mod binary;
pub use binary::*;
mod order;
pub use order::{OrderError, OrderKey};

use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

macro_rules! identifier {
    ($name:ident, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
        pub struct $name(String);
        impl $name {
            /// Constructs an opaque identifier. Writers are responsible for uniqueness.
            pub fn new(value: impl Into<String>) -> Self {
                Self(value.into())
            }
            /// Returns the opaque identifier, without a stable serialization guarantee.
            pub fn as_str(&self) -> &str {
                &self.0
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                self.0.fmt(f)
            }
        }
    };
}

identifier!(DatabaseId, "Identity of one logical database.");
identifier!(
    GenerationId,
    "Identity of one state generation; public object IDs survive recovery."
);
identifier!(GroupId, "Stable identity of a group.");
identifier!(OperationId, "Identity of one idempotent user operation.");
identifier!(EntryId, "Stable identity of an entry.");
identifier!(
    AttachmentId,
    "Identity of an attachment independent of its name."
);
identifier!(
    BlobId,
    "Opaque identity of immutable binary content within one database."
);
identifier!(AttributeId, "Stable identity of a custom attribute.");
identifier!(
    RevisionId,
    "Stable identity of one confirmed entry revision."
);

/// UTC milliseconds, used for presentation rather than conflict resolution.
pub type Timestamp = i64;

/// A current group with one unambiguous name and acyclic placement.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Group {
    /// Stable identity.
    pub id: GroupId,
    /// Exact, nonempty name.
    pub name: String,
    /// Parent, or no parent for a top-level group.
    pub parent: Option<GroupId>,
    /// Dense sibling position; compare it with the stable group ID.
    pub order: OrderKey,
    /// Current state generation.
    pub generation: GenerationId,
    /// Stored icon; reading it never accesses the network.
    pub icon: IconRef,
    /// Creation time.
    pub created_at: Timestamp,
    /// Most recent meaningful local change time.
    pub modified_at: Timestamp,
}

/// A reference to a particular generation of a group.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct GroupRef {
    /// Public identity.
    pub id: GroupId,
    /// A closed generation never redirects to a later recovery.
    pub generation: GenerationId,
}

/// Atomic parent and sibling position.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupPlacement {
    /// None denotes the top level, not an implicit root object.
    pub parent: Option<GroupRef>,
    /// Dense position with deterministic identity tie breaking.
    pub order: OrderKey,
}

/// An atomic pair: changing protection cannot expose another concurrent value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttributeValue {
    /// Exact content, including whitespace.
    pub value: String,
    /// Whether ordinary presentation must conceal this value.
    pub protected: bool,
}

/// An independently addressed custom field.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attribute {
    /// Identity retained when the attribute is renamed.
    pub id: AttributeId,
    /// Exact, nonempty name, unique within one local entry form.
    pub name: String,
    /// Atomic value and protection flag.
    pub value: AttributeValue,
}

/// Editable entry contents. Absence and an existing empty string are distinct.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntryFields {
    /// Exact, nonempty title.
    pub title: String,
    /// Optional login.
    pub username: Option<String>,
    /// Optional password, always concealed in ordinary presentation.
    pub password: Option<String>,
    /// Optional URL; reading it never fetches a resource.
    pub url: Option<String>,
    /// Optional atomic notes, without character-wise merging.
    pub notes: Option<String>,
    /// Exact, case-sensitive tags, replaced as one set.
    pub tags: BTreeSet<String>,
    /// Optional expiration instant, in UTC milliseconds.
    pub expires_at: Option<Timestamp>,
    /// Custom attributes keyed by their stable identity.
    pub attributes: BTreeMap<AttributeId, Attribute>,
    /// Independently addressed attachment references, never binary contents.
    pub attachments: BTreeMap<AttachmentId, Attachment>,
    /// Explicit presentation values; absent colors follow the UI theme.
    pub appearance: Appearance,
}

/// A structural validation failure; it never contains user-entered content.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ValidationError {
    /// A group name is empty.
    EmptyGroupName,
    /// Attachment names must be nonempty.
    EmptyAttachmentName,
    /// An attachment map key disagrees with its identity.
    AttachmentIdentityMismatch,
    /// An entry title is empty.
    EmptyTitle,
    /// An attribute name is empty.
    EmptyAttributeName,
    /// Two local attributes have exactly the same name.
    DuplicateAttributeName,
    /// A map key and its attribute's identity disagree.
    AttributeIdentityMismatch,
}

impl fmt::Display for ValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ValidationError {}

/// Validates a group name without trimming or normalizing it.
pub fn validate_group_name(name: &str) -> Result<(), ValidationError> {
    if name.is_empty() {
        Err(ValidationError::EmptyGroupName)
    } else {
        Ok(())
    }
}

impl EntryFields {
    /// Validates local form invariants without changing any supplied value.
    pub fn validate(&self) -> Result<(), ValidationError> {
        if self.title.is_empty() {
            return Err(ValidationError::EmptyTitle);
        }
        for (id, attachment) in &self.attachments {
            if id != &attachment.id {
                return Err(ValidationError::AttachmentIdentityMismatch);
            }
            if attachment.name.is_empty() {
                return Err(ValidationError::EmptyAttachmentName);
            }
        }
        let mut names = BTreeSet::new();
        for (id, attr) in &self.attributes {
            if id != &attr.id {
                return Err(ValidationError::AttributeIdentityMismatch);
            }
            if attr.name.is_empty() {
                return Err(ValidationError::EmptyAttributeName);
            }
            if !names.insert(&attr.name) {
                return Err(ValidationError::DuplicateAttributeName);
            }
        }
        Ok(())
    }
}

/// An addressed entry field, independent of an adapter's object IDs.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum EntryField {
    /// Title.
    Title,
    /// Login.
    Username,
    /// Password.
    Password,
    /// URL.
    Url,
    /// Notes.
    Notes,
    /// Entire tag set.
    Tags,
    /// Expiration instant.
    ExpiresAt,
    /// An attribute's independently editable name.
    AttributeName(AttributeId),
    /// An attribute's atomic value and protection.
    AttributeValue(AttributeId),
    /// Explicit existence, preserving concurrent deletion and edits.
    AttributePresence(AttributeId),
    /// Attachment name.
    AttachmentName(AttachmentId),
    /// Immutable content reference.
    AttachmentBlob(AttachmentId),
    /// Existence witnessed by every attachment edit.
    AttachmentPresence(AttachmentId),
    /// Icon and source form one atomic value.
    Icon,
    /// Explicit foreground color.
    Foreground,
    /// Explicit background color.
    Background,
}

/// A typed atomic field value. Debug deliberately reveals no contents.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum FieldValue {
    /// Optional text; `None` is explicit absence.
    Text(Option<String>),
    /// Entire tag set.
    Tags(BTreeSet<String>),
    /// Optional UTC instant.
    Timestamp(Option<Timestamp>),
    /// Atomic value and protection of an attribute.
    Attribute(AttributeValue),
    /// Whether an attribute is present.
    Presence(bool),
    /// Immutable binary content identity.
    Blob(BlobId),
    /// Icon and its encrypted provenance.
    Icon(IconRef),
    /// Optional sRGB RGBA color.
    Color(Option<Color>),
}

/// One displayed value and all original operations that supplied that value.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ValueVariant {
    /// Exact atomic value.
    pub value: FieldValue,
    /// Opaque source operation identifiers, not authentication credentials.
    pub origins: Vec<String>,
}

/// All visible alternatives for a field, including an unambiguous field.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldState {
    /// Addressed field.
    pub field: EntryField,
    /// Distinct values with their complete provenance.
    pub variants: Vec<ValueVariant>,
}

/// A conflicting field and every retained alternative.
pub type FieldConflict = FieldState;

/// A complete logical view, without choosing a CRDT winner.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EntrySnapshot {
    /// Stable identity.
    pub id: EntryId,
    /// Fixed placement in this increment.
    pub group_id: Option<GroupId>,
    /// State generation containing these fields.
    pub generation: GenerationId,
    /// All atomic destination alternatives, never an implicit winner.
    pub placements: Vec<GroupRef>,
    /// A regular editable form, available only when every field is unambiguous.
    pub fields: Option<EntryFields>,
    /// Conflicting fields; all alternatives remain in `values` as well.
    pub conflicts: Vec<FieldConflict>,
    /// Complete logical field state, retained even if `fields` is unavailable.
    pub values: Vec<FieldState>,
    /// Original creation time.
    pub created_at: Timestamp,
    /// Derived display time; never resolves a conflict.
    pub modified_at: Timestamp,
}

impl EntrySnapshot {
    /// Whether ordinary editing must defer to an explicit conflict workflow.
    pub fn has_conflicts(&self) -> bool {
        !self.conflicts.is_empty() || self.placements.len() != 1
    }
}

/// A supported kind of confirmed revision.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevisionKind {
    /// First confirmed entry state.
    Create,
    /// A subsequent confirmed edit.
    Save,
    /// Initial state of a clone with fresh identities.
    Clone,
    /// Explicitly restored historical state.
    Restore,
    /// Explicit resolution of observed conflicting variants.
    Resolve,
}

/// An immutable logical entry revision, committed together with its edits.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedRevision {
    /// Identity independent of the enclosing change hash.
    pub id: RevisionId,
    /// Entry to which this revision belongs.
    pub entry_id: EntryId,
    /// Confirmation time.
    pub saved_at: Timestamp,
    /// Confirmation kind.
    pub kind: RevisionKind,
    /// Original causal heads, encoded opaquely by the document adapter.
    pub base: Vec<String>,
    /// Complete logical snapshot, including unresolved alternatives.
    pub snapshot: EntrySnapshot,
}

macro_rules! redacted_debug {
    ($($name:ty),+ $(,)?) => { $(impl fmt::Debug for $name {
        fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
            f.write_str(concat!(stringify!($name), " { <redacted> }"))
        }
    })+ };
}
redacted_debug!(
    Group,
    Attachment,
    IconRef,
    Appearance,
    AttributeValue,
    Attribute,
    EntryFields,
    FieldValue,
    ValueVariant,
    FieldState,
    EntrySnapshot,
    SavedRevision
);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn validates_without_normalizing_or_requiring_a_password() {
        let mut fields = EntryFields {
            title: "  Пример  ".into(),
            password: Some("  public é\n".into()),
            ..Default::default()
        };
        assert_eq!(fields.validate(), Ok(()));
        assert_eq!(fields.password.as_deref(), Some("  public é\n"));
        fields.password = Some(String::new());
        assert_eq!(fields.validate(), Ok(()));
        fields.password = None;
        assert_eq!(fields.validate(), Ok(()));
        fields.title.clear();
        assert_eq!(fields.validate(), Err(ValidationError::EmptyTitle));
        assert_eq!(
            validate_group_name(""),
            Err(ValidationError::EmptyGroupName)
        );
    }

    #[test]
    fn attribute_names_are_exact_and_id_addressed() {
        let mut fields = EntryFields {
            title: "PUBLIC test".into(),
            ..Default::default()
        };
        for (id, name) in [("a", "Code"), ("b", "code")] {
            let id = AttributeId::new(id);
            fields.attributes.insert(
                id.clone(),
                Attribute {
                    id,
                    name: name.into(),
                    value: AttributeValue {
                        value: "PUBLIC".into(),
                        protected: true,
                    },
                },
            );
        }
        assert_eq!(fields.validate(), Ok(()));
        fields
            .attributes
            .get_mut(&AttributeId::new("b"))
            .unwrap()
            .name = "Code".into();
        assert_eq!(
            fields.validate(),
            Err(ValidationError::DuplicateAttributeName)
        );
    }

    #[test]
    fn diagnostics_do_not_contain_entry_or_attribute_contents() {
        let fields = EntryFields {
            title: "PUBLIC title marker".into(),
            password: Some("PUBLIC secret marker".into()),
            ..Default::default()
        };
        let rendered = format!("{fields:?}");
        assert!(!rendered.contains("marker"));
        assert!(rendered.contains("redacted"));
    }
}
