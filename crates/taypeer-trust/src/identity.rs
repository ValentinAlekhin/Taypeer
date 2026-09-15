use crate::{Error, canonical};
use ed25519_dalek::{Signer, SigningKey, VerifyingKey};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Deserializer, Serialize, Serializer, de::Error as _};
use sha2::{Digest as _, Sha256};
use std::{fmt, str::FromStr};
use zeroize::Zeroizing;

fn decode<const N: usize>(text: &str) -> Result<[u8; N], Error> {
    if text.len() != N * 2
        || !text
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    {
        return Err(Error::Invalid);
    }
    let mut result = [0; N];
    for (out, pair) in result.iter_mut().zip(text.as_bytes().chunks_exact(2)) {
        let digit = |b: u8| if b <= b'9' { b - b'0' } else { b - b'a' + 10 };
        *out = digit(pair[0]) * 16 + digit(pair[1]);
    }
    Ok(result)
}
fn encode(bytes: &[u8], f: &mut fmt::Formatter<'_>) -> fmt::Result {
    for byte in bytes {
        write!(f, "{byte:02x}")?;
    }
    Ok(())
}

macro_rules! fixed {
    ($name:ident, $size:expr, $doc:literal) => {
        #[doc = $doc]
        #[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
        pub struct $name(pub(crate) [u8; $size]);
        impl $name {
            /// Exact fixed-width public representation.
            pub fn as_bytes(&self) -> &[u8; $size] {
                &self.0
            }
            /// Construct a fixed-width value; signature/key validation is separate.
            pub fn from_bytes(bytes: [u8; $size]) -> Self {
                Self(bytes)
            }
        }
        impl FromStr for $name {
            type Err = Error;
            fn from_str(text: &str) -> Result<Self, Error> {
                decode(text).map(Self)
            }
        }
        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                encode(&self.0, f)
            }
        }
        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                fmt::Display::fmt(self, f)
            }
        }
        impl Serialize for $name {
            fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
                serializer.collect_str(self)
            }
        }
        impl<'de> Deserialize<'de> for $name {
            fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
                String::deserialize(deserializer)?
                    .parse()
                    .map_err(|_| D::Error::custom("invalid fixed-width value"))
            }
        }
    };
}
fixed!(
    Digest,
    32,
    "SHA-256 identifier of a domain-separated object or ciphertext."
);
fixed!(
    DeviceId,
    32,
    "Stable device identity derived from its author public key."
);
fixed!(
    TrustSetId,
    32,
    "Independent random lineage of network authority for a database."
);
fixed!(
    PublicKey,
    32,
    "Encoded Ed25519 public key, checked strictly at verification."
);
fixed!(
    Signature,
    64,
    "An Ed25519 signature; never an authorization by itself."
);

impl Digest {
    /// Compute a ciphertext digest; callers must not publish hashes of user secrets.
    pub fn of(bytes: &[u8]) -> Self {
        Self(Sha256::digest(bytes).into())
    }
    /// Hash a canonical public protocol object in its own domain.
    pub fn object(domain: &[u8], value: &impl Serialize) -> Result<Self, Error> {
        Ok(Self::of(&canonical(domain, value)?))
    }
}
pub(crate) fn random<const N: usize>() -> Result<[u8; N], Error> {
    let mut bytes = [0; N];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| Error::Random)?;
    Ok(bytes)
}
impl TrustSetId {
    /// Start a new independent lineage; never derives trust from a copied file.
    pub fn random() -> Result<Self, Error> {
        random().map(Self)
    }
}
impl PublicKey {
    /// Reject malformed/weak keys before they enter a control chain.
    pub fn validate(&self) -> Result<(), Error> {
        let key = VerifyingKey::from_bytes(&self.0).map_err(|_| Error::Invalid)?;
        if key.is_weak() {
            return Err(Error::Invalid);
        }
        Ok(())
    }
    pub(crate) fn verify(
        &self,
        domain: &[u8],
        body: &impl Serialize,
        signature: &Signature,
    ) -> Result<(), Error> {
        let key = VerifyingKey::from_bytes(&self.0).map_err(|_| Error::Signature)?;
        key.verify_strict(
            &canonical(domain, body)?,
            &ed25519_dalek::Signature::from_bytes(&signature.0),
        )
        .map_err(|_| Error::Signature)
    }
}

/// Both public roles of a profile. Display names stay in encrypted user metadata.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Identity {
    /// Public author identity, independent of transport connectivity.
    pub device: DeviceId,
    /// Signs source changes and, when authorized, control transitions.
    pub author: PublicKey,
    /// Authenticates QUIC and ciphertext manifests only.
    pub transport: PublicKey,
}
impl Identity {
    /// Bind independent keys; equal role keys are rejected.
    pub fn new(author: PublicKey, transport: PublicKey) -> Result<Self, Error> {
        let identity = Self {
            device: device_id(author),
            author,
            transport,
        };
        identity.validate()?;
        Ok(identity)
    }
    /// Validate a deserialized identity without issuing any admission.
    pub fn validate(&self) -> Result<(), Error> {
        self.author.validate()?;
        self.transport.validate()?;
        if self.author == self.transport || self.device != device_id(self.author) {
            return Err(Error::Invalid);
        }
        Ok(())
    }
}

/// Author secret, owned by an unlocked session or initial profile enrollment.
/// Debug and serialization are deliberately unavailable.
pub struct AuthorKey(SigningKey);
/// Transport secret; holding it does not authorize source changes or management.
pub struct TransportKey(SigningKey);
macro_rules! secret {
    ($name:ident) => {
        impl $name {
            /// Generate an independent secret from the operating system RNG.
            pub fn generate() -> Result<Self, Error> {
                let seed = Zeroizing::new(random()?);
                Ok(Self(SigningKey::from_bytes(&seed)))
            }
            /// Load a seed obtained through the platform credential adapter.
            pub fn from_seed(seed: &[u8; 32]) -> Self {
                Self(SigningKey::from_bytes(seed))
            }
            /// Export only for the platform credential adapter or transport constructor.
            pub fn secret_seed(&self) -> Zeroizing<[u8; 32]> {
                Zeroizing::new(self.0.to_bytes())
            }
            /// Public half used in a signed device binding.
            pub fn public(&self) -> PublicKey {
                PublicKey(self.0.verifying_key().to_bytes())
            }
            pub(crate) fn sign(
                &self,
                domain: &[u8],
                body: &impl Serialize,
            ) -> Result<Signature, Error> {
                Ok(Signature(self.0.sign(&canonical(domain, body)?).to_bytes()))
            }
        }
    };
}
secret!(AuthorKey);
secret!(TransportKey);
impl AuthorKey {
    /// Identity of this author; transport keys have no equivalent author capability.
    pub fn device_id(&self) -> DeviceId {
        device_id(self.public())
    }
}
fn device_id(public: PublicKey) -> DeviceId {
    let mut body = b"taypeer/device/1".to_vec();
    body.extend(public.0);
    DeviceId(Digest::of(&body).0)
}
