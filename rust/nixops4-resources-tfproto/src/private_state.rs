//! Encoding of tfplugin's opaque `private` bytes as the JSON value
//! NixOps4 persists in a resource's `privateState` slot.
//!
//! The NixOps4 slot is `Option<serde_json::Value>` (opaque to it); the
//! tfplugin field is `bytes`. Empty bytes and `None` are the same
//! semantically ("no private state"), so both round-trip through
//! `None`. Non-empty bytes are base64-encoded into a JSON string —
//! chosen over a marker object because the bytes are already opaque
//! and a bare string keeps the state file free of adapter-internal
//! shape.

use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::Value;

/// Encode raw tfplugin private bytes as the value NixOps4 stores.
/// Empty bytes become `None` (nothing to persist).
pub fn encode(bytes: Vec<u8>) -> Option<Value> {
    if bytes.is_empty() {
        return None;
    }
    Some(Value::String(
        base64::engine::general_purpose::STANDARD.encode(bytes),
    ))
}

/// Decode a previously-persisted NixOps4 private-state value back to
/// raw tfplugin bytes. `None` — or a JSON `null` — decodes to empty
/// bytes, matching tfplugin's "no prior private" case.
pub fn decode(value: Option<Value>) -> Result<Vec<u8>> {
    match value {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::String(s)) => base64::engine::general_purpose::STANDARD
            .decode(&s)
            .context("decoding privateState as base64"),
        Some(other) => anyhow::bail!(
            "privateState must be a base64 string (this adapter's shape); got {}",
            other
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn empty_bytes_encode_to_none_and_decode_from_none() {
        assert_eq!(encode(Vec::new()), None);
        assert_eq!(decode(None).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn round_trip_preserves_arbitrary_bytes() {
        let bytes = vec![0u8, 1, 2, 255, b'a', b'z', 0x7f, 0x80];
        let encoded = encode(bytes.clone()).unwrap();
        assert_eq!(decode(Some(encoded)).unwrap(), bytes);
    }

    #[test]
    fn json_null_is_treated_as_absent() {
        assert_eq!(decode(Some(Value::Null)).unwrap(), Vec::<u8>::new());
    }

    #[test]
    fn non_string_value_errors_cleanly() {
        let err = decode(Some(serde_json::json!({"unexpected": "shape"}))).unwrap_err();
        assert!(err.to_string().contains("base64 string"), "got: {err}");
    }
}
