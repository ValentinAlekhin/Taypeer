//! Binary references and presentation values, without I/O or cryptography.

use crate::{AttachmentId, BlobId};
use serde::{Deserialize, Serialize};

/// One MiB, used for product limits rather than decimal megabytes.
pub const MIB: u64 = 1024 * 1024;
/// Default maximum bytes in one new attachment.
pub const ATTACHMENT_LIMIT: u64 = 10 * MIB;
/// Default unique retained attachment bytes in one database.
pub const DATABASE_ATTACHMENT_LIMIT: u64 = 100 * MIB;
/// Maximum original bytes in a user-supplied icon.
pub const ICON_LIMIT: u64 = MIB;

/// An independently named reference to immutable content.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Attachment {
    /// Stable identity across renaming and replacement.
    pub id: AttachmentId,
    /// Exact nonempty name; duplicates are allowed.
    pub name: String,
    /// Content identity, shared across names and revisions.
    pub blob: BlobId,
}

/// An explicit sRGB RGBA value, independent of the application theme.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Color(pub [u8; 4]);

/// A key in the bundled Lucide catalog. Deserialization validates membership.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct LucideKey(String);

/// Invalid built-in icon identity; contains no user input.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct InvalidIcon;

macro_rules! lucide_catalog {
    ($($key:literal),+ $(,)?) => {
        /// Stable keys shared by the CLI and platform icon pickers.
        pub const LUCIDE_KEYS: &[&str] = &[$($key),+];
        impl LucideKey {
            /// Bundled original SVG. Loading an icon never accesses the network.
            pub fn svg(&self) -> &'static str {
                match self.0.as_str() {
                    $($key => include_str!(concat!("../../../resources/lucide/", $key, ".svg")),)+
                    _ => unreachable!("LucideKey validates catalog membership"),
                }
            }
        }
    };
}
lucide_catalog!(
    "key-round",
    "lock",
    "globe",
    "mail",
    "user",
    "users",
    "folder",
    "folder-open",
    "file",
    "file-key-2",
    "shield",
    "credit-card",
    "wallet",
    "server",
    "database",
    "terminal",
    "smartphone",
    "laptop",
    "home",
    "star",
    "heart",
    "bookmark",
);

/// Upstream attribution shipped alongside the embedded catalog.
pub const LUCIDE_LICENSE: &str = include_str!("../../../resources/lucide/LICENSE");

impl TryFrom<String> for LucideKey {
    type Error = InvalidIcon;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if LUCIDE_KEYS.contains(&value.as_str()) {
            Ok(Self(value))
        } else {
            Err(InvalidIcon)
        }
    }
}
impl From<LucideKey> for String {
    fn from(value: LucideKey) -> Self {
        value.0
    }
}
impl std::fmt::Display for InvalidIcon {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid icon")
    }
}
impl std::error::Error for InvalidIcon {}

/// Stored icon; its source is provenance, never a request to fetch on read.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IconRef {
    /// Standard folder or entry icon chosen by the presentation.
    #[default]
    Default,
    /// Bundled vector icon with a stable key.
    Lucide(LucideKey),
    /// Validated original image stored in the database.
    Image {
        /// Immutable image bytes.
        blob: BlobId,
        /// Explicit acquisition origin; stored under database encryption.
        source: IconSource,
    },
}

/// Acquisition origin, kept out of diagnostics.
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum IconSource {
    /// User-selected file; no local path is persisted.
    File,
    /// Explicit image URL.
    Url(String),
    /// Page URL from which the user requested a favicon.
    Favicon(String),
}

/// Independent editable presentation fields.
#[derive(Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Appearance {
    /// Stored icon and acquisition origin.
    pub icon: IconRef,
    /// None follows the application theme.
    pub foreground: Option<Color>,
    /// None follows the application theme.
    pub background: Option<Color>,
}

impl IconRef {
    /// Returns the stored binary reference, if this is a custom image.
    pub fn blob(&self) -> Option<&BlobId> {
        match self {
            Self::Image { blob, .. } => Some(blob),
            _ => None,
        }
    }
}
