//! The adapter's use of nixops4's `privateState` slot.
//!
//! Two things live there, as one JSON object:
//!
//! * `provider`: the opaque `private` bytes a tfplugin provider
//!   returns from Apply and expects back on the next Plan (base64).
//! * `state`: the provider's complete `new_state` from the last Apply.
//!   Terraform keeps exactly this as the resource's state and hands it
//!   back as `prior_state`. nixops4 keeps inputs and computed outputs
//!   instead, and reconstructing prior state from those loses what the
//!   provider filled in on its own: SDK defaults for optional
//!   attributes, for one. The Scaleway provider decides whether to
//!   delete a server's root volume from `delete_on_termination` in
//!   prior state, which it defaulted to true and which the
//!   reconstruction turned into null; the volume survived the server.
//!
//! A bare base64 string is the shape this adapter wrote before `state`
//! existed; it still decodes, with no stored state.

use anyhow::{Context, Result};
use base64::Engine as _;
use serde_json::{Map, Value};

#[derive(Debug, Default, PartialEq)]
pub struct Private {
    pub provider: Vec<u8>,
    pub state: Option<Value>,
}

/// Encode for the `privateState` slot; `None` when there is nothing
/// to keep, so nixops4 stores no slot at all.
pub fn encode(private: Private) -> Option<Value> {
    if private.provider.is_empty() && private.state.is_none() {
        return None;
    }
    let mut obj = Map::new();
    obj.insert("v".to_string(), Value::from(1));
    if !private.provider.is_empty() {
        obj.insert(
            "provider".to_string(),
            Value::String(base64::engine::general_purpose::STANDARD.encode(private.provider)),
        );
    }
    if let Some(state) = private.state {
        obj.insert("state".to_string(), state);
    }
    Some(Value::Object(obj))
}

pub fn decode(value: Option<Value>) -> Result<Private> {
    match value {
        None | Some(Value::Null) => Ok(Private::default()),
        Some(Value::String(s)) => Ok(Private {
            provider: decode_provider(&s)?,
            state: None,
        }),
        Some(Value::Object(mut obj)) => {
            let provider = match obj.remove("provider") {
                None | Some(Value::Null) => Vec::new(),
                Some(Value::String(s)) => decode_provider(&s)?,
                Some(other) => anyhow::bail!(
                    "privateState.provider must be a base64 string; got {}",
                    other
                ),
            };
            let state = match obj.remove("state") {
                None | Some(Value::Null) => None,
                Some(v) => Some(v),
            };
            Ok(Private { provider, state })
        }
        Some(other) => anyhow::bail!(
            "privateState must be an object or a base64 string (this adapter's shapes); got {}",
            other
        ),
    }
}

fn decode_provider(s: &str) -> Result<Vec<u8>> {
    base64::engine::general_purpose::STANDARD
        .decode(s)
        .context("decoding privateState provider bytes as base64")
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn nothing_encodes_to_none_and_decodes_from_none() {
        assert_eq!(encode(Private::default()), None);
        assert_eq!(decode(None).unwrap(), Private::default());
        assert_eq!(decode(Some(Value::Null)).unwrap(), Private::default());
    }

    #[test]
    fn round_trip_keeps_bytes_and_state() {
        let private = Private {
            provider: vec![0u8, 1, 2, 255, b'a', 0x7f, 0x80],
            state: Some(json!({"id": "x", "delete_on_termination": true})),
        };
        let encoded = encode(Private {
            provider: private.provider.clone(),
            state: private.state.clone(),
        })
        .unwrap();
        assert_eq!(encoded["v"], json!(1));
        assert_eq!(decode(Some(encoded)).unwrap(), private);
    }

    #[test]
    fn state_alone_is_kept() {
        let encoded = encode(Private {
            provider: Vec::new(),
            state: Some(json!({"id": "x"})),
        })
        .unwrap();
        assert_eq!(encoded, json!({"v": 1, "state": {"id": "x"}}));
    }

    #[test]
    fn legacy_base64_string_still_decodes() {
        let encoded = Value::String(base64::engine::general_purpose::STANDARD.encode(b"cookie"));
        assert_eq!(
            decode(Some(encoded)).unwrap(),
            Private {
                provider: b"cookie".to_vec(),
                state: None
            }
        );
    }

    #[test]
    fn other_shapes_error_cleanly() {
        let err = decode(Some(json!([1, 2]))).unwrap_err();
        assert_eq!(
            err.to_string(),
            "privateState must be an object or a base64 string (this adapter's shapes); got [1,2]"
        );
    }
}
