//! Pure compatibility rules. Authentication and durable receipt belong to their callers.
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Current experimental document contract, not a stable-format release.
pub const CURRENT_SCHEMA: u16 = 5;
/// Semantic requirements of the current document, including its retention rules.
pub const DOCUMENT_FEATURES: [&str; 4] = [
    "taypeer.entries",
    "taypeer.history",
    "taypeer.lifecycle",
    "taypeer.binary",
];

/// A bounded, nonsecret semantic feature identifier with a stable meaning.
#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(try_from = "String")]
pub struct FeatureId(String);
impl FeatureId {
    /// Checked identifiers are safe to include in public compatibility diagnostics.
    pub fn new(value: impl Into<String>) -> Result<Self, DescriptorError> {
        let value = value.into();
        if value.is_empty()
            || value.len() > 96
            || !value
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"._-".contains(&b))
        {
            return Err(DescriptorError);
        }
        Ok(Self(value))
    }
    /// Stable wire spelling, independent of translated labels.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}
impl TryFrom<String> for FeatureId {
    type Error = DescriptorError;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        Self::new(value)
    }
}

/// Invalid or incomplete schema declaration; contains no document contents.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct DescriptorError;
impl std::fmt::Display for DescriptorError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("invalid schema descriptor")
    }
}
impl std::error::Error for DescriptorError {}

/// Manager-authenticated requirements, immutable throughout the current control chain.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "DescriptorData")]
pub struct SchemaDescriptor {
    schema_version: u16,
    required_read_features: BTreeSet<FeatureId>,
    required_write_features: BTreeSet<FeatureId>,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct DescriptorData {
    schema_version: u16,
    required_read_features: BTreeSet<FeatureId>,
    required_write_features: BTreeSet<FeatureId>,
}
impl TryFrom<DescriptorData> for SchemaDescriptor {
    type Error = DescriptorError;
    fn try_from(value: DescriptorData) -> Result<Self, Self::Error> {
        Self::new(
            value.schema_version,
            value.required_read_features,
            value.required_write_features,
        )
    }
}
impl SchemaDescriptor {
    /// Validate shape and mandatory requirements of known schemas. Unknown schemas
    /// can be carried as ciphertext; construction does not imply readable contents.
    pub fn new(
        schema_version: u16,
        required_read_features: BTreeSet<FeatureId>,
        required_write_features: BTreeSet<FeatureId>,
    ) -> Result<Self, DescriptorError> {
        if schema_version == 0
            || required_read_features.len() > 128
            || required_write_features.len() > 128
        {
            return Err(DescriptorError);
        }
        if schema_version == CURRENT_SCHEMA {
            let required = document_features();
            if !required.is_subset(&required_read_features)
                || !required.is_subset(&required_write_features)
            {
                return Err(DescriptorError);
            }
        }
        Ok(Self {
            schema_version,
            required_read_features,
            required_write_features,
        })
    }
    /// The schema written by this release. No source-file migration is implied.
    pub fn current() -> Self {
        Self {
            schema_version: CURRENT_SCHEMA,
            required_read_features: document_features(),
            required_write_features: document_features(),
        }
    }
    /// Independent logical schema version.
    pub fn schema_version(&self) -> u16 {
        self.schema_version
    }
    /// Semantics needed to expose decrypted contents safely.
    pub fn required_read_features(&self) -> &BTreeSet<FeatureId> {
        &self.required_read_features
    }
    /// Semantics needed to author changes or collect contents safely.
    pub fn required_write_features(&self) -> &BTreeSet<FeatureId> {
        &self.required_write_features
    }
}
fn document_features() -> BTreeSet<FeatureId> {
    DOCUMENT_FEATURES
        .into_iter()
        .map(|id| FeatureId(id.into()))
        .collect()
}

/// Supported semantics of a client build. Restrictions can remove capabilities;
/// callers cannot claim a reader or writer that is absent from the implementation.
#[derive(Clone, Debug)]
pub struct ClientCapabilities {
    read: BTreeSet<FeatureId>,
    write: BTreeSet<FeatureId>,
}
impl Default for ClientCapabilities {
    fn default() -> Self {
        Self {
            read: document_features(),
            write: document_features(),
        }
    }
}
impl ClientCapabilities {
    /// Restrict a client to a subset of its compiled capabilities.
    pub fn restricted(mut self, read: &BTreeSet<FeatureId>, write: &BTreeSet<FeatureId>) -> Self {
        self.read.retain(|feature| read.contains(feature));
        self.write.retain(|feature| write.contains(feature));
        self
    }
    /// Assess a descriptor from an already verified, supported outer container/control
    /// encoding. Network admission, lock state and author permission are separate.
    pub fn assess(&self, descriptor: &SchemaDescriptor) -> CompatibilityReport {
        let read = if descriptor.schema_version != CURRENT_SCHEMA {
            CompatibilityAccess::UnsupportedSchema {
                schema_version: descriptor.schema_version,
            }
        } else {
            missing(&descriptor.required_read_features, &self.read)
        };
        let write = if read.is_supported() {
            missing(&descriptor.required_write_features, &self.write)
        } else {
            CompatibilityAccess::ReadingUnavailable
        };
        CompatibilityReport {
            schema: descriptor.clone(),
            read,
            write,
            receive: CompatibilityAccess::Supported,
        }
    }
}
fn missing(required: &BTreeSet<FeatureId>, available: &BTreeSet<FeatureId>) -> CompatibilityAccess {
    let features: BTreeSet<_> = required.difference(available).cloned().collect();
    if features.is_empty() {
        CompatibilityAccess::Supported
    } else {
        CompatibilityAccess::MissingFeatures { features }
    }
}

/// One format-level capability; contains no authentication or session state.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "status", rename_all = "snake_case")]
pub enum CompatibilityAccess {
    /// Supported by this build.
    Supported,
    /// No decoder exists for this logical schema.
    UnsupportedSchema {
        /// Unsupported logical schema, not an application version.
        schema_version: u16,
    },
    /// The representation is known but mandatory semantics are unavailable.
    MissingFeatures {
        /// Stable identifiers suitable for public diagnostics.
        features: BTreeSet<FeatureId>,
    },
    /// Writing requires a supported reader as well as writer semantics.
    ReadingUnavailable,
}
impl CompatibilityAccess {
    /// Whether this specific format capability is supported.
    pub fn is_supported(&self) -> bool {
        matches!(self, Self::Supported)
    }
}
/// Independent read, write and durable ciphertext-receipt capabilities.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CompatibilityReport {
    /// Authenticated schema requirements.
    pub schema: SchemaDescriptor,
    /// Ability to open the decrypted document.
    pub read: CompatibilityAccess,
    /// Ability to change its logical state, including cleanup and administration.
    pub write: CompatibilityAccess,
    /// Ability to carry supported ciphertext envelopes; never a promise of admission or ACK.
    pub receive: CompatibilityAccess,
}

#[cfg(test)]
mod tests;
