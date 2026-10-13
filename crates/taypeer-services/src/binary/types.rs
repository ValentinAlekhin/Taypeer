//! Binary service requests and safe metadata responses.
use crate::{FieldUpdate, InspectionTarget, ServiceError};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use taypeer_core::{
    AttachmentId, BlobId, Color, DraftId, EntryId, GroupId, IconRef, IconSource, LucideKey,
    OperationId, RevisionId,
};

/// Metadata of an explicitly selected attachment stream, independent of its
/// transient platform capability. The complete declared length is authenticated.
pub struct AttachmentImport {
    /// Exact complete plaintext length.
    pub length: u64,
    /// Exact selected filename for a new attachment.
    pub name: String,
    /// Existing identity when changing only its immutable content.
    pub replacement: Option<AttachmentId>,
}

/// Original image from one explicit selection. The operation freezes its source;
/// an acknowledged retry never acquires the provider or network again.
pub struct IconImport {
    /// Complete plaintext length, authenticated with exact EOF while reading.
    pub length: u64,
    /// Encrypted provenance; a file source never includes a host path or URI.
    pub source: IconSource,
}

/// One already-held favicon original, scoped to its exact causal form.
#[derive(Clone, Serialize, Deserialize)]
pub struct StoredFavicon {
    /// Immutable original identity; a bare identity grants no read authority.
    pub identity: BlobId,
    /// Verified available bytes, absent when encrypted content is still missing.
    pub bytes: Option<u64>,
}

/// Explicit scope of binary metadata and export; purged objects require a late source.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum BinaryTarget {
    /// Current active entry.
    Entry(EntryId),
    /// Current active group.
    Group(GroupId),
    /// Selected icon of one accessible immutable group confirmation.
    GroupRevision {
        /// Owning group identity.
        group: GroupId,
        /// Exact saved confirmation.
        revision: RevisionId,
    },
    /// Active unconfirmed form.
    Draft,
    /// Exact causal entry or group form, including a retained parked editor.
    Form(DraftId),
    /// The currently offered local favicon original for this exact form.
    FormFavicon {
        /// Owning causal form.
        draft: DraftId,
        /// Exact original returned by stored_favicon; changed candidates reject.
        identity: BlobId,
    },
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

/// Separate product quota, retained contents, local draft and working-file usage.
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

/// Bound shared with Android's metadata adapter; original image bytes use a stream.
pub const FAVICON_PLAN_LIMIT: usize = 1024 * 1024;

/// One explicit group action. A new selection requires a new operation identity.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GroupFaviconRequest {
    /// Active selected group.
    pub group: GroupId,
    /// Include descendant groups when explicitly selected.
    pub recursive: bool,
    /// Replace custom icons only when explicitly selected.
    pub replace: bool,
    /// Parent operation; freezes group and options before any network request.
    pub operation: OperationId,
}

/// Exact child request retained by a live batch. URL is never logged.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct FaviconEntryRequest {
    /// Original group action, including its operation identity.
    pub batch: GroupFaviconRequest,
    /// Saved entry selected by the shared service.
    pub entry: EntryId,
    /// Exact ordinary URL captured by preparation; no network occurs on decoding.
    pub url: String,
    /// Deterministically derived child identity; callers cannot substitute it.
    pub operation: OperationId,
}

/// Shared selection result; the platform does not repeat group/icon filtering.
#[derive(Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum FaviconPlanAction {
    /// Keep an existing custom icon unchanged.
    Skipped,
    /// No ordinary URL exists for this saved entry.
    MissingUrl,
    /// Acquire only after the child receipt preflight permits this exact request.
    Acquire(FaviconEntryRequest),
}

/// One fixed row of a live-session batch; it contains no protected entry fields.
#[derive(Clone, Serialize, Deserialize)]
pub struct FaviconPlanRow {
    /// Saved entry identity.
    pub entry: EntryId,
    /// Ordinary label captured for displaying this operation's result.
    pub title: String,
    /// Fixed shared selection.
    pub action: FaviconPlanAction,
}

/// Bounded preparation, not a cold-resume journal or an all-or-nothing transaction.
#[derive(Clone, Serialize, Deserialize)]
pub struct GroupFaviconPlan {
    /// Original exact group/options/operation.
    pub request: GroupFaviconRequest,
    /// Fixed per-entry actions within this live attempt.
    pub rows: Vec<FaviconPlanRow>,
}

/// Validated local image bytes for a bounded inline preview; never fetched from the network.
#[derive(Clone, Serialize, Deserialize)]
pub struct IconPreview {
    /// Opaque stored identity.
    pub blob: BlobId,
    /// Validated image encoding.
    pub encoding: IconEncoding,
    /// At most the product icon limit; erases this allocation on drop.
    pub bytes: zeroize::Zeroizing<Vec<u8>>,
}
/// Image encodings accepted by the shared icon validator.
#[derive(Clone, Copy, Serialize, Deserialize)]
pub enum IconEncoding {
    /// PNG raster.
    Png,
    /// JPEG raster.
    Jpeg,
    /// WebP raster.
    Webp,
    /// GIF raster.
    Gif,
    /// ICO raster.
    Ico,
    /// SVG without external resources.
    Svg,
}
