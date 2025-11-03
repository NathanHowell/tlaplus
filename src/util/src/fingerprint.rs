use blake3::Hasher;
use serde::{Deserialize, Serialize};
use std::convert::TryFrom;
use std::fmt;
use thiserror::Error;

const FINGERPRINT_LEN: usize = 16;
const CHECKSUM_LEN: usize = 4;

/// Errors that can arise while decoding or validating fingerprints.
#[derive(Debug, Error)]
pub enum FingerprintError {
    #[error("fingerprint byte slice must be exactly {expected} bytes (got {actual})")]
    InvalidLength { expected: usize, actual: usize },
    #[error("invalid fingerprint hex string: {0}")]
    InvalidHex(String),
}

/// Canonical TLC state fingerprint: 128-bit value with metadata for validation/debugging.
#[derive(Clone, Copy, PartialEq, Eq, Hash)]
pub struct StateFingerprint {
    value: u128,
    generation: u64,
    checksum: u32,
}

impl StateFingerprint {
    /// Creates a fingerprint from raw parts.
    pub fn from_parts(value: u128, generation: u64, checksum: u32) -> Self {
        Self {
            value,
            generation,
            checksum,
        }
    }

    /// Computes a new fingerprint for the provided canonical state bytes.
    pub fn new(state_bytes: &[u8], generation: u64) -> Self {
        FingerprintBuilder::default()
            .update(state_bytes)
            .finalize(generation)
    }

    /// Returns the 128-bit fingerprint value.
    pub fn value(&self) -> u128 {
        self.value
    }

    /// Returns the exploration generation/layer where this fingerprint was first observed.
    pub fn generation(&self) -> u64 {
        self.generation
    }

    /// Returns the checksum derived from the canonical hash.
    pub fn checksum(&self) -> u32 {
        self.checksum
    }

    /// Encodes the fingerprint value as 16 big-endian bytes.
    pub fn to_bytes(&self) -> [u8; FINGERPRINT_LEN] {
        self.value.to_be_bytes()
    }

    /// Attempts to parse a fingerprint from big-endian bytes.
    pub fn from_bytes(
        bytes: &[u8],
        generation: u64,
        checksum: u32,
    ) -> Result<Self, FingerprintError> {
        if bytes.len() != FINGERPRINT_LEN {
            return Err(FingerprintError::InvalidLength {
                expected: FINGERPRINT_LEN,
                actual: bytes.len(),
            });
        }
        let mut buf = [0u8; FINGERPRINT_LEN];
        buf.copy_from_slice(bytes);
        Ok(Self::from_parts(
            u128::from_be_bytes(buf),
            generation,
            checksum,
        ))
    }

    /// Returns the hexadecimal string representation of the fingerprint value.
    pub fn to_hex(&self) -> String {
        format!("{:032x}", self.value)
    }

    /// Parses a fingerprint value from a hexadecimal string.
    pub fn from_hex(hex: &str, generation: u64, checksum: u32) -> Result<Self, FingerprintError> {
        let value = u128::from_str_radix(hex, 16)
            .map_err(|_| FingerprintError::InvalidHex(hex.to_owned()))?;
        Ok(Self::from_parts(value, generation, checksum))
    }
}

impl fmt::Debug for StateFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateFingerprint")
            .field("value", &format_args!("{:032x}", self.value))
            .field("generation", &self.generation)
            .field("checksum", &format_args!("{:08x}", self.checksum))
            .finish()
    }
}

#[derive(Serialize, Deserialize)]
struct FingerprintSerde {
    #[serde(with = "hex_u128")]
    value: u128,
    generation: u64,
    checksum: u32,
}

impl Serialize for StateFingerprint {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        FingerprintSerde {
            value: self.value,
            generation: self.generation,
            checksum: self.checksum,
        }
        .serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for StateFingerprint {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let raw = FingerprintSerde::deserialize(deserializer)?;
        Ok(Self::from_parts(raw.value, raw.generation, raw.checksum))
    }
}

/// Incremental helper for building deterministic state fingerprints.
pub struct FingerprintBuilder {
    hasher: Hasher,
}

impl FingerprintBuilder {
    /// Creates a new builder with an optional domain separation tag.
    pub fn with_context(context: &[u8]) -> Self {
        let mut hasher = Hasher::new();
        if !context.is_empty() {
            let len = u64::try_from(context.len()).expect("context length overflow");
            hasher.update(&len.to_be_bytes());
            hasher.update(context);
        }
        Self { hasher }
    }

    /// Appends raw bytes to the fingerprint stream (builder chaining variant).
    pub fn update(mut self, bytes: &[u8]) -> Self {
        self.update_ref(bytes);
        self
    }

    /// Appends raw bytes to the fingerprint stream (mutable reference variant).
    pub fn update_ref(&mut self, bytes: &[u8]) -> &mut Self {
        let len = u64::try_from(bytes.len()).expect("fingerprint input length overflow");
        self.hasher.update(&len.to_be_bytes());
        self.hasher.update(bytes);
        self
    }

    /// Appends a u64 value in big-endian representation.
    pub fn update_u64(self, value: u64) -> Self {
        self.update(&value.to_be_bytes())
    }

    /// Appends a u128 value in big-endian representation.
    pub fn update_u128(self, value: u128) -> Self {
        self.update(&value.to_be_bytes())
    }

    /// Finalizes the accumulated data into a `StateFingerprint`.
    pub fn finalize(self, generation: u64) -> StateFingerprint {
        let hash = self.hasher.finalize();
        let bytes = hash.as_bytes();

        let mut fingerprint_bytes = [0u8; FINGERPRINT_LEN];
        fingerprint_bytes.copy_from_slice(&bytes[..FINGERPRINT_LEN]);

        let mut checksum_bytes = [0u8; CHECKSUM_LEN];
        checksum_bytes.copy_from_slice(&bytes[FINGERPRINT_LEN..FINGERPRINT_LEN + CHECKSUM_LEN]);

        StateFingerprint::from_parts(
            u128::from_be_bytes(fingerprint_bytes),
            generation,
            u32::from_be_bytes(checksum_bytes),
        )
    }
}

impl Default for FingerprintBuilder {
    fn default() -> Self {
        Self {
            hasher: Hasher::new(),
        }
    }
}

mod hex_u128 {
    use super::FingerprintError;
    use serde::{Deserialize, Deserializer, Serializer};

    pub fn serialize<S>(value: &u128, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(&format!("{:032x}", value))
    }

    pub fn deserialize<'de, D>(deserializer: D) -> Result<u128, D::Error>
    where
        D: Deserializer<'de>,
    {
        let hex = String::deserialize(deserializer)?;
        u128::from_str_radix(&hex, 16)
            .map_err(|_| serde::de::Error::custom(FingerprintError::InvalidHex(hex).to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprints_are_deterministic() {
        let bytes = b"canonical-state-representation";
        let fingerprint_a = StateFingerprint::new(bytes, 5);
        let fingerprint_b = StateFingerprint::new(bytes, 5);
        assert_eq!(fingerprint_a.value(), fingerprint_b.value());
        assert_eq!(fingerprint_a.checksum(), fingerprint_b.checksum());
    }

    #[test]
    fn builder_sequence_matches_direct_hash() {
        let bytes = b"state-chunk";
        let generation = 42;
        let direct = StateFingerprint::new(bytes, generation);
        let via_builder = FingerprintBuilder::default()
            .update(bytes)
            .finalize(generation);
        assert_eq!(direct.value(), via_builder.value());
        assert_eq!(direct.checksum(), via_builder.checksum());
    }

    #[test]
    fn serialization_round_trip() {
        let fingerprint = FingerprintBuilder::with_context(b"ctx")
            .update(b"payload")
            .update_u64(123)
            .update_u128(456)
            .finalize(7);

        let json = serde_json::to_string(&fingerprint).expect("serialize");
        let back: StateFingerprint = serde_json::from_str(&json).expect("deserialize");

        assert_eq!(fingerprint.value(), back.value());
        assert_eq!(fingerprint.generation(), back.generation());
        assert_eq!(fingerprint.checksum(), back.checksum());
    }

    #[test]
    fn from_bytes_validates_length() {
        let fingerprint = StateFingerprint::new(b"hello", 0);
        let bytes = fingerprint.to_bytes();
        let parsed =
            StateFingerprint::from_bytes(&bytes, fingerprint.generation(), fingerprint.checksum())
                .expect("parse");
        assert_eq!(parsed.value(), fingerprint.value());

        let err = StateFingerprint::from_bytes(&bytes[..4], 0, 0).unwrap_err();
        assert!(matches!(err, FingerprintError::InvalidLength { .. }));
    }
}
