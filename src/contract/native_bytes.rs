//! Byte payload that crosses the Rust/JS boundary as a `Uint8Array`.

use std::fmt;
use std::sync::Arc;

use serde::de::{self, SeqAccess, Visitor};
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use super::codec::cautious_capacity;
use super::{TsSchema, TsType};

/// Byte payload that crosses the typed host bridge as a native-backed `Uint8Array`.
///
/// `NativeBytes` is an explicit opt-in for large byte payloads. The native codec hands
/// JavaScript an immutable `Uint8Array` backed by the Rust allocation (no copy) and
/// reads a `Uint8Array`, `ArrayBuffer` or array of bytes back with one copy. The
/// JSON-compatible fallback (serde) writes a byte array and reads bytes or a byte array.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativeBytes {
    bytes: Arc<[u8]>,
}

impl NativeBytes {
    /// Creates a native byte payload from owned bytes.
    #[must_use]
    pub fn new(bytes: impl Into<Vec<u8>>) -> Self {
        Self {
            bytes: bytes.into().into(),
        }
    }

    /// Creates a native byte payload from shared bytes.
    #[must_use]
    pub fn from_shared(bytes: Arc<[u8]>) -> Self {
        Self { bytes }
    }

    /// Returns the bytes as a slice.
    #[must_use]
    pub fn as_slice(&self) -> &[u8] {
        self.bytes.as_ref()
    }

    /// Returns the shared backing bytes.
    #[must_use]
    pub fn into_shared(self) -> Arc<[u8]> {
        self.bytes
    }
}

impl From<Vec<u8>> for NativeBytes {
    fn from(bytes: Vec<u8>) -> Self {
        Self::new(bytes)
    }
}

impl From<Box<[u8]>> for NativeBytes {
    fn from(bytes: Box<[u8]>) -> Self {
        Self {
            bytes: Arc::from(bytes),
        }
    }
}

impl From<Arc<[u8]>> for NativeBytes {
    fn from(bytes: Arc<[u8]>) -> Self {
        Self::from_shared(bytes)
    }
}

impl AsRef<[u8]> for NativeBytes {
    fn as_ref(&self) -> &[u8] {
        self.as_slice()
    }
}

impl Serialize for NativeBytes {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_bytes(self.as_slice())
    }
}

impl<'de> Deserialize<'de> for NativeBytes {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_bytes(NativeBytesVisitor)
    }
}

/// Accepts serde bytes or a sequence of `u8`, which is how `serde_json` writes bytes.
struct NativeBytesVisitor;

impl<'de> Visitor<'de> for NativeBytesVisitor {
    type Value = NativeBytes;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("bytes or an array of integers in 0..=255")
    }

    fn visit_bytes<E: de::Error>(self, bytes: &[u8]) -> Result<NativeBytes, E> {
        Ok(NativeBytes::from_shared(Arc::from(bytes)))
    }

    fn visit_byte_buf<E: de::Error>(self, bytes: Vec<u8>) -> Result<NativeBytes, E> {
        Ok(NativeBytes::from(bytes))
    }

    fn visit_seq<A: SeqAccess<'de>>(self, mut sequence: A) -> Result<NativeBytes, A::Error> {
        let capacity = cautious_capacity::<u8>(sequence.size_hint().unwrap_or(0));
        let mut bytes = Vec::with_capacity(capacity);
        while let Some(byte) = sequence.next_element::<u8>()? {
            bytes.push(byte);
        }
        Ok(NativeBytes::from(bytes))
    }
}

impl TsSchema for NativeBytes {
    fn schema_name() -> Option<&'static str> {
        Some("NativeBytes")
    }

    fn ts_type() -> TsType {
        TsType::Uint8Array
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn deserializes_the_byte_array_it_serializes() {
        let bytes = NativeBytes::new(vec![0, 7, 255]);
        let json = serde_json::to_value(&bytes).expect("serialize bytes");

        assert_eq!(json, json!([0, 7, 255]));
        assert_eq!(
            serde_json::from_value::<NativeBytes>(json).expect("deserialize bytes"),
            bytes
        );
    }

    #[test]
    fn rejects_values_outside_the_byte_range() {
        assert!(serde_json::from_value::<NativeBytes>(json!([256])).is_err());
        assert!(serde_json::from_value::<NativeBytes>(json!({ "bytes": [1] })).is_err());
    }
}
