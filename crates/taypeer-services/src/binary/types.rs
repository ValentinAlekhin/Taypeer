//! Binary service requests and safe metadata responses.
use crate::{FieldUpdate, InspectionTarget, ServiceError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use taypeer_core::{AttachmentId, BlobId, Color, EntryId, GroupId, IconRef, LucideKey, RevisionId};

/// Explicit scope of binary metadata and export; purged objects require a late source.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum BinaryTarget {
    /// Current active entry.
    Entry(EntryId),
    /// Current active group.
    Group(GroupId),
    /// Active unconfirmed form.
    Draft,
    /// Accessible saved revision.
    Revision {
        /// Owning entry.
        entry: EntryId,
        /// Selected revision.
        revision: RevisionId,
    },
    /// Retained trash generation or late source.
    Inspection(InspectionTarget),
}

/// An explicit attachment action; source paths are read by the worker and retained only in encrypted retry intents.
#[derive(Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case")]
pub enum AttachmentEdit {
    /// Add a new identity with streamed contents.
    Add {
        /// Explicit selected source.
        path: PathBuf,
        /// Exact name, defaulting to the source basename.
        name: Option<String>,
    },
    /// Change only the name.
    Rename {
        /// Stable identity.
        attachment: AttachmentId,
        /// Exact nonempty name.
        name: String,
    },
    /// Change only immutable content, retaining the name and identity.
    Replace {
        /// Stable identity.
        attachment: AttachmentId,
        /// Selected replacement file.
        path: PathBuf,
    },
    /// Remove from the current form; accessible history continues to retain its contents.
    Remove {
        /// Stable identity.
        attachment: AttachmentId,
    },
}

/// Explicit acquisition of an icon; nothing is fetched by deserialization or reads.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IconInput {
    /// Use the standard icon.
    Default,
    /// Stable bundled Lucide key.
    Lucide(LucideKey),
    /// Selected local image.
    File(PathBuf),
    /// Explicit original image URL.
    Url(String),
    /// Page URL, or the entry's current URL when omitted.
    Favicon(Option<String>),
}

/// Binary or presentation changes, sharing the ordinary entry draft.
#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum BinaryEdit {
    /// One independently addressed attachment change.
    Attachment(AttachmentEdit),
    /// Icon and provenance form one atomic field.
    Icon(IconInput),
    /// Independently addressed explicit colors.
    Appearance {
        /// Omitted preserves the existing foreground.
        #[serde(default)]
        foreground: FieldUpdate<Color>,
        /// Omitted preserves the existing background.
        #[serde(default)]
        background: FieldUpdate<Color>,
    },
}

/// A retryable binary command; ordinary entry targets confirm once, Draft only stages.
#[derive(Serialize, Deserialize)]
pub struct BinaryRequest {
    /// Entry, group or active draft; historical targets are read-only.
    pub target: BinaryTarget,
    /// Explicit requested action.
    pub edit: BinaryEdit,
    /// Reviewed heads when resolving group icon variants.
    pub review: Option<Vec<String>>,
}

/// Locally verified availability, without a content fingerprint.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BlobAvailability {
    /// Opaque binary identity.
    pub id: BlobId,
    /// Verified bytes; None means the selected source awaits contents.
    pub bytes: Option<u64>,
}
/// Every retained name/content variant of one attachment.
#[derive(Clone, Serialize, Deserialize)]
pub struct AttachmentView {
    /// Stable identity.
    pub id: AttachmentId,
    /// Complete name alternatives.
    pub names: Vec<String>,
    /// Complete content alternatives and availability.
    pub contents: Vec<BlobAvailability>,
    /// Deletion competes with an edit; explicit resolution is required.
    pub deletion_conflict: bool,
}
/// Binary-only metadata; no passwords, protected attributes or file contents.
#[derive(Clone, Serialize, Deserialize)]
pub struct BinaryView {
    /// Attachments and all retained variants.
    pub attachments: Vec<AttachmentView>,
    /// Stored icon alternatives.
    pub icons: Vec<IconRef>,
    /// Optional color alternatives; the inner None follows the theme.
    pub foreground: Vec<Option<Color>>,
    /// Optional background alternatives.
    pub background: Vec<Option<Color>>,
}

/// Separate product quota, retained contents, local draft and backup usage.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct StorageUsage {
    /// Unique accessible attachment bytes.
    pub attachment_bytes: u64,
    /// Current product quota.
    pub attachment_limit: u64,
    /// Accepted data may exceed the quota after merging.
    pub over_limit: bool,
    /// Unique contents retained by the document and pending sources.
    pub retained_bytes: u64,
    /// Separate logical contents owned by the local draft.
    pub draft_bytes: u64,
    /// Physical encrypted working-file size.
    pub file_bytes: u64,
    /// Physical separate automatic backup size.
    pub backup_bytes: u64,
    /// Missing contents referenced by retained sources.
    pub missing: Vec<BlobId>,
    /// Unknown pending references conservatively prevent deletion.
    pub unknown_references: bool,
}

/// Per-entry batch outcome; successful siblings survive an acquisition failure.
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct FaviconResult {
    /// Entry selected by the batch.
    pub entry: EntryId,
    /// Whether the existing icon was deliberately left unchanged.
    pub skipped: bool,
    /// Categorized acquisition or persistence error, without response contents.
    pub error: Option<ServiceError>,
}
