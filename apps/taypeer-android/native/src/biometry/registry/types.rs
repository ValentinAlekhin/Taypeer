//! Validated, canonical metadata for one copy's OS-owned biometric wrapper.
use super::RegistryError;
use base64::{Engine as _, engine::general_purpose::STANDARD};
use serde::{Deserialize, Serialize};
use std::{fmt, path::Path};
use taypeer_core::DatabaseId;
use taypeer_trust::Digest;

pub(super) const MAX_RECORD_BYTES: usize = 4096;
const SCHEME: &str = "rsa_oaep_sha256_mgf1_sha1";

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::biometry) struct Binding {
    namespace: Digest,
    working_copy: Digest,
    #[serde(deserialize_with = "deserialize_database")]
    database: DatabaseId,
    root: Digest,
    epoch: u64,
}
impl Binding {
    /// Bind a registered copy to its canonical private profile. No path is stored.
    pub(in crate::biometry) fn for_profile(
        profile: &Path,
        working_copy: Digest,
        database: DatabaseId,
        root: Digest,
        epoch: u64,
    ) -> Result<Self, RegistryError> {
        let profile = profile.canonicalize().map_err(|_| RegistryError::Io)?;
        let namespace = Digest::object(
            b"taypeer.android.biometry.profile.v1",
            &profile.as_os_str().as_encoded_bytes(),
        )
        .map_err(|_| RegistryError::Invalid)?;
        let binding = Self {
            namespace,
            working_copy,
            database,
            root,
            epoch,
        };
        binding.validate()?;
        Ok(binding)
    }
    fn validate(&self) -> Result<(), RegistryError> {
        // Defensive projection of the shared trust control.rs DatabaseId rule.
        // DatabaseId's existing serde wrapper does not enforce its wire boundary.
        validate_database(&self.database)
    }
    pub(in crate::biometry) fn copy_digest(&self) -> Result<Digest, RegistryError> {
        Digest::object(
            b"taypeer.android.biometry.copy.v1",
            &(self.namespace, self.working_copy, &self.database, self.root),
        )
        .map_err(|_| RegistryError::Invalid)
    }
    pub(in crate::biometry) fn permanent_digest(&self) -> Result<Digest, RegistryError> {
        Digest::object(b"taypeer.android.biometry.binding.v1", self)
            .map_err(|_| RegistryError::Invalid)
    }
    pub(super) fn same_copy(&self, other: &Self) -> bool {
        self.namespace == other.namespace
            && self.working_copy == other.working_copy
            && self.database == other.database
            && self.root == other.root
    }
    pub(super) fn same_profile(&self, profile: &Path) -> Result<bool, RegistryError> {
        Ok(self.namespace
            == Self::for_profile(
                profile,
                self.working_copy,
                self.database.clone(),
                self.root,
                self.epoch,
            )?
            .namespace)
    }
    pub(in crate::biometry) fn namespace(&self) -> Digest {
        self.namespace
    }
    pub(in crate::biometry) fn working_copy(&self) -> Digest {
        self.working_copy
    }
    pub(in crate::biometry) fn database(&self) -> &DatabaseId {
        &self.database
    }
    pub(in crate::biometry) fn root(&self) -> Digest {
        self.root
    }
    pub(in crate::biometry) fn epoch(&self) -> u64 {
        self.epoch
    }
}
fn validate_database(database: &DatabaseId) -> Result<(), RegistryError> {
    if database.as_str().is_empty() || database.as_str().len() > 128 {
        return Err(RegistryError::Invalid);
    }
    Ok(())
}
fn deserialize_database<'de, D: serde::Deserializer<'de>>(
    deserializer: D,
) -> Result<DatabaseId, D::Error> {
    let database = DatabaseId::deserialize(deserializer)?;
    validate_database(&database).map_err(serde::de::Error::custom)?;
    Ok(database)
}
impl fmt::Debug for Binding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Binding([redacted])")
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub(in crate::biometry) struct Generation([u8; 16]);
impl Generation {
    /// The fixed OS adapter owns random generation; this checks its exact encoding.
    pub(in crate::biometry) fn parse(value: &str) -> Result<Self, RegistryError> {
        if value.len() != 32
            || !value
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
        {
            return Err(RegistryError::Invalid);
        }
        let mut bytes = [0; 16];
        for (byte, pair) in bytes.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
            fn digit(byte: u8) -> u8 {
                if byte.is_ascii_digit() {
                    byte - b'0'
                } else {
                    byte - b'a' + 10
                }
            }
            *byte = digit(pair[0]) * 16 + digit(pair[1]);
        }
        Ok(Self(bytes))
    }
}
impl fmt::Display for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl fmt::Debug for Generation {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Generation([redacted])")
    }
}
impl Serialize for Generation {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}
impl<'de> Deserialize<'de> for Generation {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        Self::parse(&String::deserialize(deserializer)?).map_err(serde::de::Error::custom)
    }
}

#[derive(Clone, PartialEq, Eq)]
struct WrappedCiphertext([u8; 256]);
impl Serialize for WrappedCiphertext {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&STANDARD.encode(self.0))
    }
}
impl<'de> Deserialize<'de> for WrappedCiphertext {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let encoded = String::deserialize(deserializer)?;
        if encoded.len() != 344 {
            return Err(serde::de::Error::custom("invalid wrapped ciphertext"));
        }
        let decoded = STANDARD
            .decode(&encoded)
            .map_err(|_| serde::de::Error::custom("invalid wrapped ciphertext"))?;
        if STANDARD.encode(&decoded) != encoded {
            return Err(serde::de::Error::custom("invalid wrapped ciphertext"));
        }
        let bytes = decoded
            .try_into()
            .map_err(|_| serde::de::Error::custom("invalid wrapped ciphertext"))?;
        Ok(Self(bytes))
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "StoredWrapper")]
pub(in crate::biometry) struct WrapperRecord {
    binding: Binding,
    generation: Generation,
    alias: String,
    spki_digest: Digest,
    ciphertext: WrappedCiphertext,
}
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct StoredWrapper {
    binding: Binding,
    generation: Generation,
    alias: String,
    spki_digest: Digest,
    ciphertext: WrappedCiphertext,
}
impl TryFrom<StoredWrapper> for WrapperRecord {
    type Error = RegistryError;
    fn try_from(stored: StoredWrapper) -> Result<Self, Self::Error> {
        let record = Self::new(
            stored.binding,
            stored.generation,
            stored.spki_digest,
            stored.ciphertext.0,
        )?;
        if record.alias != stored.alias {
            return Err(RegistryError::Invalid);
        }
        Ok(record)
    }
}
impl WrapperRecord {
    pub(in crate::biometry) fn new(
        binding: Binding,
        generation: Generation,
        spki_digest: Digest,
        ciphertext: [u8; 256],
    ) -> Result<Self, RegistryError> {
        binding.validate()?;
        let alias = Self::scoped_alias(&binding, generation)?;
        Ok(Self {
            binding,
            generation,
            alias,
            spki_digest,
            ciphertext: WrappedCiphertext(ciphertext),
        })
    }
    pub(in crate::biometry) fn scoped_alias(
        binding: &Binding,
        generation: Generation,
    ) -> Result<String, RegistryError> {
        Ok(format!(
            "taypeer_biometry_v1_{}_{}",
            binding.permanent_digest()?,
            generation
        ))
    }
    fn validate(&self, binding: &Binding) -> Result<(), RegistryError> {
        self.binding.validate()?;
        if !self.binding.same_copy(binding)
            || self.alias != Self::scoped_alias(&self.binding, self.generation)?
        {
            return Err(RegistryError::Invalid);
        }
        Ok(())
    }
    pub(in crate::biometry) fn binding(&self) -> &Binding {
        &self.binding
    }
    pub(in crate::biometry) fn generation(&self) -> Generation {
        self.generation
    }
    pub(in crate::biometry) fn alias(&self) -> &str {
        &self.alias
    }
    pub(in crate::biometry) fn spki_digest(&self) -> Digest {
        self.spki_digest
    }
    pub(in crate::biometry) fn ciphertext(&self) -> &[u8; 256] {
        &self.ciphertext.0
    }
}
impl fmt::Debug for WrapperRecord {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("WrapperRecord([redacted])")
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum ProofStage {
    ProofConfirmed,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "action", rename_all = "snake_case", deny_unknown_fields)]
pub(super) enum PendingSetup {
    Enable {
        operation: Digest,
        candidate: Box<WrapperRecord>,
        stage: ProofStage,
    },
    Disable {
        operation: Digest,
        generation: Generation,
    },
}
impl PendingSetup {
    pub(super) fn enable(operation: Digest, candidate: WrapperRecord) -> Self {
        Self::Enable {
            operation,
            candidate: Box::new(candidate),
            stage: ProofStage::ProofConfirmed,
        }
    }
    pub(super) fn operation(&self) -> Digest {
        match self {
            Self::Enable { operation, .. } | Self::Disable { operation, .. } => *operation,
        }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub(super) enum SetupAction {
    Enable,
    Disable,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct SetupReceipt {
    pub(super) operation: Digest,
    pub(super) action: SetupAction,
    pub(super) generation: Generation,
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct Record {
    version: u16,
    scheme: String,
    pub(super) active: Option<WrapperRecord>,
    pub(super) pending: Option<PendingSetup>,
    pub(super) receipt: Option<SetupReceipt>,
}
impl Record {
    pub(super) fn empty() -> Self {
        Self {
            version: 1,
            scheme: SCHEME.into(),
            active: None,
            pending: None,
            receipt: None,
        }
    }
    fn validate(&self, binding: &Binding) -> Result<(), RegistryError> {
        binding.validate()?;
        if self.version != 1 || self.scheme != SCHEME {
            return Err(RegistryError::Invalid);
        }
        if let Some(active) = &self.active {
            active.validate(binding)?;
        }
        if self.active.is_some() && self.receipt.is_none() {
            return Err(RegistryError::Invalid);
        }
        if let Some(pending) = &self.pending {
            if self
                .receipt
                .as_ref()
                .is_some_and(|receipt| receipt.operation == pending.operation())
            {
                return Err(RegistryError::Invalid);
            }
            match pending {
                PendingSetup::Enable { candidate, .. } => {
                    candidate.validate(binding)?;
                    if self
                        .active
                        .as_ref()
                        .is_some_and(|active| active.generation == candidate.generation)
                        || self
                            .receipt
                            .as_ref()
                            .is_some_and(|receipt| receipt.generation == candidate.generation)
                    {
                        return Err(RegistryError::Invalid);
                    }
                }
                PendingSetup::Disable { generation, .. } => {
                    if !self
                        .active
                        .as_ref()
                        .is_some_and(|active| active.generation == *generation)
                    {
                        return Err(RegistryError::Invalid);
                    }
                }
            }
        }
        if let Some(receipt) = &self.receipt {
            match receipt.action {
                SetupAction::Enable
                    if !self
                        .active
                        .as_ref()
                        .is_some_and(|active| active.generation == receipt.generation) =>
                {
                    return Err(RegistryError::Invalid);
                }
                SetupAction::Disable if self.active.is_some() => {
                    return Err(RegistryError::Invalid);
                }
                SetupAction::Enable | SetupAction::Disable => {}
            }
        }
        Ok(())
    }
    pub(super) fn encode(&self, binding: &Binding) -> Result<Vec<u8>, RegistryError> {
        self.validate(binding)?;
        let bytes = serde_json::to_vec(self).map_err(|_| RegistryError::Invalid)?;
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(RegistryError::Invalid);
        }
        Ok(bytes)
    }
    pub(super) fn decode(bytes: &[u8], binding: &Binding) -> Result<Self, RegistryError> {
        if bytes.len() > MAX_RECORD_BYTES {
            return Err(RegistryError::Invalid);
        }
        let record: Self = serde_json::from_slice(bytes).map_err(|_| RegistryError::Invalid)?;
        // Canonical field order/encodings are part of exact-byte publication replay.
        if record.encode(binding)? != bytes {
            return Err(RegistryError::Invalid);
        }
        Ok(record)
    }
}
impl fmt::Debug for Record {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Record([redacted])")
    }
}
