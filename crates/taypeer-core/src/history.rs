//! Immutable selected states of descriptive objects; no CRDT or storage dependency.

use crate::{DatabaseId, Group, GroupId, RevisionId, RevisionKind, Timestamp};
use serde::{Deserialize, Serialize};

/// Selected state of a group at one explicit save.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupSnapshot {
    /// Identity, placement and presentation of the saved group.
    pub group: Group,
    /// Exact optional descriptive text.
    pub description: Option<String>,
}

/// A confirmed group version; automatic merging creates no version.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedGroupRevision {
    /// Stable confirmation identity.
    pub id: RevisionId,
    /// Owning group.
    pub group_id: GroupId,
    /// Display timestamp, never a winner rank.
    pub saved_at: Timestamp,
    /// Confirmation kind.
    pub kind: RevisionKind,
    /// Original causal context.
    pub base: Vec<String>,
    /// Selected state; original branch versions preserve alternatives separately.
    pub snapshot: GroupSnapshot,
}

/// Selected descriptive state of a database at one explicit save.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DatabaseSnapshot {
    /// Owning logical database.
    pub database_id: DatabaseId,
    /// Exact nonempty display name.
    pub name: String,
    /// Exact optional descriptive text.
    pub description: Option<String>,
}

/// A confirmed database name/description version.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct SavedDatabaseRevision {
    /// Stable confirmation identity.
    pub id: RevisionId,
    /// Display timestamp, never a winner rank.
    pub saved_at: Timestamp,
    /// Confirmation kind.
    pub kind: RevisionKind,
    /// Original causal context.
    pub base: Vec<String>,
    /// Selected state.
    pub snapshot: DatabaseSnapshot,
}

macro_rules! redacted_debug {
    ($($name:ty),+ $(,)?) => { $(impl std::fmt::Debug for $name {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            f.write_str(concat!(stringify!($name), " { <redacted> }"))
        }
    })+ };
}
redacted_debug!(
    GroupSnapshot,
    SavedGroupRevision,
    DatabaseSnapshot,
    SavedDatabaseRevision
);
