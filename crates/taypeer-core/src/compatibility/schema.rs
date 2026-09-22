//! Compiled schema contracts, independent of the version chosen for new databases.
//! Development definitions may be removed before v1; released stable definitions may not.
use super::{BTreeSet, FeatureId};

pub(super) struct Definition {
    pub(super) version: u16,
    read: &'static [&'static str],
    write: &'static [&'static str],
}
impl Definition {
    pub(super) fn read_features(&self) -> BTreeSet<FeatureId> {
        features(self.read)
    }
    pub(super) fn write_features(&self) -> BTreeSet<FeatureId> {
        features(self.write)
    }
}
fn features(ids: &[&str]) -> BTreeSet<FeatureId> {
    ids.iter().map(|id| FeatureId((*id).into())).collect()
}
const DEVELOPMENT_V5_FEATURES: [&str; 4] = [
    "taypeer.entries",
    "taypeer.history",
    "taypeer.lifecycle",
    "taypeer.binary",
];
const DEVELOPMENT_V5: Definition = Definition {
    version: 5,
    read: &DEVELOPMENT_V5_FEATURES,
    write: &DEVELOPMENT_V5_FEATURES,
};
// Adding a new writer does not implicitly remove any compiled reader.
const SUPPORTED: &[Definition] = &[DEVELOPMENT_V5];
/// Semantic requirements written by this build; retained for API compatibility.
pub const DOCUMENT_FEATURES: [&str; 4] = DEVELOPMENT_V5_FEATURES;
/// Schema written by this build; not a stable-format release or the reader support list.
pub const CURRENT_SCHEMA: u16 = DEVELOPMENT_V5.version;

pub(super) fn current() -> &'static Definition {
    &DEVELOPMENT_V5
}
pub(super) fn definition(version: u16) -> Option<&'static Definition> {
    SUPPORTED
        .iter()
        .find(|definition| definition.version == version)
}
pub(super) fn supported_read_features() -> BTreeSet<FeatureId> {
    SUPPORTED
        .iter()
        .flat_map(Definition::read_features)
        .collect()
}
pub(super) fn supported_write_features() -> BTreeSet<FeatureId> {
    SUPPORTED
        .iter()
        .flat_map(Definition::write_features)
        .collect()
}
