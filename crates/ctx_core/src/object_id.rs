//! Object identification and canonical envelope format.

use crate::error::{CtxError, Result};
use serde::{Deserialize, Serialize};
use std::fmt;

/// A 32-byte BLAKE3 content hash used to identify objects.
///
/// ObjectIds are the foundation of CTX's content-addressed storage.
/// The same content always produces the same ObjectId, enabling
/// deduplication and integrity verification.
///
/// # Examples
///
/// ```
/// use ctx_core::ObjectId;
///
/// let id = ObjectId::from_bytes([0xab; 32]);
/// assert_eq!(id.as_hex().len(), 64);
/// assert_eq!(id.shard(), "ab");
/// ```
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ObjectId([u8; 32]);

impl ObjectId {
    /// The length of an ObjectId in bytes.
    pub const LEN: usize = 32;

    /// The length of an ObjectId as a hex string.
    pub const HEX_LEN: usize = 64;

    /// Creates an ObjectId from raw bytes.
    #[inline]
    pub fn from_bytes(bytes: [u8; 32]) -> Self {
        Self(bytes)
    }

    /// Returns a reference to the underlying 32-byte BLAKE3 hash.
    ///
    /// Use this when you need direct access to the hash bytes for low-level
    /// operations or serialization.
    #[inline]
    pub fn as_bytes(&self) -> &[u8; 32] {
        &self.0
    }

    /// Returns this ObjectId as a lowercase hex string.
    ///
    /// The returned string is always exactly 64 characters long.
    pub fn as_hex(&self) -> String {
        hex::encode(self.0)
    }

    /// Returns the shard prefix (first 2 hex characters).
    ///
    /// Used for directory sharding in the object store:
    /// `.ctx/objects/{shard}/{full_hex}`
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_core::ObjectId;
    ///
    /// let mut bytes = [0u8; 32];
    /// bytes[0] = 0xab;
    /// let id = ObjectId::from_bytes(bytes);
    /// assert_eq!(id.shard(), "ab");
    /// ```
    pub fn shard(&self) -> String {
        hex::encode(&self.0[..1])
    }

    /// Parses an ObjectId from a hex string.
    ///
    /// # Errors
    ///
    /// Returns `CtxError::InvalidHex` if the string is not valid hex
    /// or is not exactly 64 characters long.
    ///
    /// # Examples
    ///
    /// ```
    /// use ctx_core::ObjectId;
    ///
    /// let hex = "a".repeat(64);
    /// let id = ObjectId::from_hex(&hex).unwrap();
    /// assert_eq!(id.as_hex(), hex);
    /// ```
    pub fn from_hex(s: &str) -> Result<Self> {
        let s = s.trim();
        if s.len() != Self::HEX_LEN {
            return Err(CtxError::InvalidHex(format!(
                "expected {} hex chars, got {}",
                Self::HEX_LEN,
                s.len()
            )));
        }

        let bytes = hex::decode(s).map_err(|e| CtxError::InvalidHex(e.to_string()))?;

        let arr: [u8; 32] = bytes
            .try_into()
            .map_err(|_| CtxError::InvalidHex("invalid length".to_string()))?;

        Ok(Self(arr))
    }

    /// Computes the ObjectId for raw bytes (blob).
    pub(crate) fn hash_blob(data: &[u8]) -> Self {
        let canonical = canonical_bytes(ObjectKind::Blob, data);
        Self::hash_canonical(&canonical)
    }

    /// Computes the ObjectId for typed/serialized data.
    pub(crate) fn hash_typed(serialized: &[u8]) -> Self {
        let canonical = canonical_bytes(ObjectKind::Typed, serialized);
        Self::hash_canonical(&canonical)
    }

    /// Computes the BLAKE3 hash of canonical bytes.
    fn hash_canonical(canonical: &[u8]) -> Self {
        let hash = blake3::hash(canonical);
        Self::from_bytes(*hash.as_bytes())
    }
}

impl fmt::Display for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.as_hex())
    }
}

impl fmt::Debug for ObjectId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ObjectId({}...)", &self.as_hex()[..12])
    }
}

/// Object kind discriminant for the canonical envelope.
#[repr(u8)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum ObjectKind {
    /// Raw bytes (file contents, logs, etc.)
    Blob = 1,
    /// Serialized typed object (commits, edges, etc.)
    Typed = 2,
}

/// Canonical envelope magic bytes.
pub(crate) const MAGIC: &[u8; 5] = b"CTXO1";

/// Constructs canonical bytes for hashing.
///
/// Format:
/// - Magic: "CTXO1" (5 bytes)
/// - Kind: u8 (1 byte)
/// - Length: u64 LE (8 bytes)
/// - Payload: variable bytes
pub(crate) fn canonical_bytes(kind: ObjectKind, payload: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(5 + 1 + 8 + payload.len());
    out.extend_from_slice(MAGIC);
    out.push(kind as u8);
    out.extend_from_slice(&(payload.len() as u64).to_le_bytes());
    out.extend_from_slice(payload);
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_from_bytes_roundtrip() {
        let bytes = [42u8; 32];
        let id = ObjectId::from_bytes(bytes);
        assert_eq!(id.as_bytes(), &bytes);
    }

    #[test]
    fn test_hex_roundtrip() {
        let mut bytes = [0u8; 32];
        for (i, b) in bytes.iter_mut().enumerate() {
            *b = (i % 256) as u8;
        }

        let id = ObjectId::from_bytes(bytes);
        let hex = id.as_hex();
        assert_eq!(hex.len(), 64);

        let parsed = ObjectId::from_hex(&hex).unwrap();
        assert_eq!(id, parsed);
    }

    #[test]
    fn test_shard() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0xab;
        let id = ObjectId::from_bytes(bytes);
        assert_eq!(id.shard(), "ab");
    }

    #[test]
    fn test_shard_leading_zero() {
        let mut bytes = [0u8; 32];
        bytes[0] = 0x05;
        let id = ObjectId::from_bytes(bytes);
        assert_eq!(id.shard(), "05");
    }

    #[test]
    fn test_from_hex_invalid_length() {
        let result = ObjectId::from_hex("abc");
        assert!(matches!(result, Err(CtxError::InvalidHex(_))));
    }

    #[test]
    fn test_from_hex_invalid_chars() {
        let result = ObjectId::from_hex(&"g".repeat(64));
        assert!(matches!(result, Err(CtxError::InvalidHex(_))));
    }

    #[test]
    fn test_from_hex_whitespace_trimmed() {
        let hex = "a".repeat(64);
        let with_whitespace = format!("  {}  ", hex);
        let id = ObjectId::from_hex(&with_whitespace).unwrap();
        assert_eq!(id.as_hex(), hex);
    }

    #[test]
    fn test_display() {
        let bytes = [0xab; 32];
        let id = ObjectId::from_bytes(bytes);
        assert_eq!(format!("{}", id), "ab".repeat(32));
    }

    #[test]
    fn test_debug_short() {
        let bytes = [0xab; 32];
        let id = ObjectId::from_bytes(bytes);
        let debug = format!("{:?}", id);
        assert!(debug.contains("abababababab")); // First 12 chars
        assert!(!debug.contains(&"ab".repeat(32))); // Not full hash
    }

    #[test]
    fn test_hash_blob_deterministic() {
        let data = b"test data";
        let id1 = ObjectId::hash_blob(data);
        let id2 = ObjectId::hash_blob(data);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_hash_blob_different_content() {
        let id1 = ObjectId::hash_blob(b"content 1");
        let id2 = ObjectId::hash_blob(b"content 2");
        assert_ne!(id1, id2);
    }

    #[test]
    fn test_canonical_bytes_format() {
        let payload = b"test";
        let canonical = canonical_bytes(ObjectKind::Blob, payload);

        // Check magic
        assert_eq!(&canonical[..5], MAGIC);

        // Check kind
        assert_eq!(canonical[5], ObjectKind::Blob as u8);

        // Check length
        let len = u64::from_le_bytes(canonical[6..14].try_into().unwrap());
        assert_eq!(len, 4);

        // Check payload
        assert_eq!(&canonical[14..], payload);
    }

    #[test]
    fn test_canonical_bytes_typed() {
        let payload = b"serialized data";
        let canonical = canonical_bytes(ObjectKind::Typed, payload);

        assert_eq!(&canonical[..5], MAGIC);
        assert_eq!(canonical[5], ObjectKind::Typed as u8);

        let len = u64::from_le_bytes(canonical[6..14].try_into().unwrap());
        assert_eq!(len, payload.len() as u64);
    }

    #[test]
    fn test_object_id_equality() {
        let bytes = [0x42; 32];
        let id1 = ObjectId::from_bytes(bytes);
        let id2 = ObjectId::from_bytes(bytes);
        assert_eq!(id1, id2);
    }

    #[test]
    fn test_object_id_inequality() {
        let id1 = ObjectId::from_bytes([0x42; 32]);
        let id2 = ObjectId::from_bytes([0x43; 32]);
        assert_ne!(id1, id2);
    }
}
