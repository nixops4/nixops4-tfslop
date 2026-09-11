//! tfplugin `DynamicValue` encoding.
//!
//! The protocol lets a `DynamicValue` carry a go-cty value as msgpack
//! or as JSON. Which one a provider actually reads depends on its SDK:
//!
//! * terraform-plugin-sdk v2 (`helper/schema/grpc_provider.go`) calls
//!   `msgpack.Unmarshal(req.Config.MsgPack, …)` and never looks at the
//!   JSON field. Configs, prior/proposed/planned states: all msgpack.
//!   It also answers in msgpack only. The Scaleway provider is one of
//!   these (muxed through `tf5to6server`).
//! * terraform-plugin-go / plugin-framework decode msgpack when it is
//!   present and fall back to JSON otherwise.
//!
//! Terraform core itself sends msgpack only. The adapter sends both:
//! msgpack because that is what every provider reads, and JSON because
//! nothing forbids it and the fixture provider reads the JSON field
//! directly. On the way back, msgpack wins when present.
//!
//! The transcoding is schema-free. go-cty's msgpack form is plain
//! msgpack for everything Nix can express (null, bool, number, string,
//! list, map/object), so a JSON document maps one to one. Two cty
//! specifics cannot be represented and are refused:
//!
//! * unknown values (ext type 0, refinements ext type 12) never occur
//!   in a fully applied `new_state`, and Nix cannot produce them;
//! * a number that fits neither int64/uint64 nor float64 is encoded by
//!   cty as a decimal string. It decodes here as a JSON string, which
//!   the Nix side would reject at the option type. Not seen in
//!   practice; left as is until a provider produces one.

use anyhow::{bail, Context, Result};
use rmpv::Value as Mp;
use serde_json::{Map, Number, Value};

use crate::tfplugin6::DynamicValue;

/// Build a `DynamicValue` from go-cty JSON bytes, carrying both the
/// msgpack encoding and the JSON as given.
pub fn from_json(json: Vec<u8>) -> Result<DynamicValue> {
    let value: Value =
        serde_json::from_slice(&json).context("parsing go-cty JSON before msgpack encoding")?;
    let mp = json_to_msgpack(&value);
    let mut msgpack = Vec::new();
    rmpv::encode::write_value(&mut msgpack, &mp).context("encoding go-cty value as msgpack")?;
    Ok(DynamicValue { msgpack, json })
}

/// Decode a `DynamicValue` into a JSON value. msgpack takes precedence
/// when both are set, mirroring plugin-go's own `Unmarshal`.
pub fn to_json(dv: &DynamicValue) -> Result<Value> {
    if !dv.msgpack.is_empty() {
        let mp = rmpv::decode::read_value(&mut &dv.msgpack[..])
            .context("decoding msgpack DynamicValue")?;
        msgpack_to_json(&mp, "")
    } else if !dv.json.is_empty() {
        serde_json::from_slice(&dv.json).context("parsing JSON DynamicValue")
    } else {
        bail!("empty DynamicValue: neither msgpack nor json is set")
    }
}

/// [`to_json`], re-serialised to bytes for callers that consume go-cty
/// JSON.
pub fn to_json_bytes(dv: &DynamicValue) -> Result<Vec<u8>> {
    serde_json::to_vec(&to_json(dv)?).context("serialising decoded DynamicValue")
}

fn json_to_msgpack(v: &Value) -> Mp {
    match v {
        Value::Null => Mp::Nil,
        Value::Bool(b) => Mp::Boolean(*b),
        Value::Number(n) => {
            if let Some(i) = n.as_i64() {
                Mp::Integer(i.into())
            } else if let Some(u) = n.as_u64() {
                Mp::Integer(u.into())
            } else {
                // serde_json guarantees one of the three accessors
                // succeeds for a finite number.
                Mp::F64(n.as_f64().expect("serde_json number is finite"))
            }
        }
        Value::String(s) => Mp::String(s.as_str().into()),
        Value::Array(items) => Mp::Array(items.iter().map(json_to_msgpack).collect()),
        Value::Object(fields) => Mp::Map(
            fields
                .iter()
                .map(|(k, v)| (Mp::String(k.as_str().into()), json_to_msgpack(v)))
                .collect(),
        ),
    }
}

fn msgpack_to_json(v: &Mp, path: &str) -> Result<Value> {
    let here = || {
        if path.is_empty() {
            "<root>".to_string()
        } else {
            path.to_string()
        }
    };
    Ok(match v {
        Mp::Nil => Value::Null,
        Mp::Boolean(b) => Value::Bool(*b),
        Mp::Integer(i) => {
            if let Some(n) = i.as_i64() {
                Value::Number(n.into())
            } else if let Some(n) = i.as_u64() {
                Value::Number(n.into())
            } else {
                bail!("{}: msgpack integer out of range", here())
            }
        }
        Mp::F32(f) => float(f64::from(*f), &here())?,
        Mp::F64(f) => float(*f, &here())?,
        Mp::String(s) => match s.as_str() {
            Some(s) => Value::String(s.to_string()),
            None => bail!("{}: msgpack string is not valid UTF-8", here()),
        },
        Mp::Binary(_) => bail!("{}: msgpack binary has no go-cty JSON form", here()),
        Mp::Array(items) => Value::Array(
            items
                .iter()
                .enumerate()
                .map(|(i, item)| msgpack_to_json(item, &format!("{}[{}]", here(), i)))
                .collect::<Result<_>>()?,
        ),
        Mp::Map(entries) => {
            let mut out = Map::new();
            for (k, item) in entries {
                let key = match k.as_str() {
                    Some(k) => k.to_string(),
                    None => bail!("{}: msgpack map key is not a string", here()),
                };
                let child = if path.is_empty() {
                    key.clone()
                } else {
                    format!("{}.{}", path, key)
                };
                out.insert(key, msgpack_to_json(item, &child)?);
            }
            Value::Object(out)
        }
        Mp::Ext(0, _) | Mp::Ext(12, _) => bail!(
            "{}: provider returned an unknown value; unknowns only exist during planning and \
             cannot appear in an applied state",
            here()
        ),
        Mp::Ext(t, _) => bail!("{}: unsupported msgpack extension type {}", here(), t),
    })
}

fn float(f: f64, path: &str) -> Result<Value> {
    match Number::from_f64(f) {
        Some(n) => Ok(Value::Number(n)),
        None => bail!("{}: non-finite number {} has no JSON form", path, f),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn round_trip(v: Value) -> Value {
        let dv = from_json(serde_json::to_vec(&v).unwrap()).unwrap();
        assert!(!dv.msgpack.is_empty());
        // Decode from msgpack alone, to prove that half is complete.
        let only_msgpack = DynamicValue {
            msgpack: dv.msgpack,
            json: Vec::new(),
        };
        to_json(&only_msgpack).unwrap()
    }

    #[test]
    fn nested_document_survives_msgpack() {
        let doc = json!({
            "name": "smoke",
            "disabled": null,
            "tags": ["a", "b"],
            "count": 3,
            "big": 9_007_199_254_740_993u64,
            "negative": -7,
            "ratio": 1.5,
            "flag": true,
            "nested": {"list": [{"port": 22}], "empty": {}},
            "ünïcödé": "ünïcödé"
        });
        assert_eq!(round_trip(doc.clone()), doc);
    }

    #[test]
    fn null_document_round_trips() {
        assert_eq!(round_trip(Value::Null), Value::Null);
    }

    #[test]
    fn json_is_kept_verbatim() {
        let bytes = b"{\"a\":1}".to_vec();
        let dv = from_json(bytes.clone()).unwrap();
        assert_eq!(dv.json, bytes);
    }

    #[test]
    fn msgpack_wins_over_json() {
        let dv = DynamicValue {
            msgpack: from_json(b"{\"from\":\"msgpack\"}".to_vec())
                .unwrap()
                .msgpack,
            json: b"{\"from\":\"json\"}".to_vec(),
        };
        assert_eq!(to_json(&dv).unwrap(), json!({"from": "msgpack"}));
    }

    #[test]
    fn json_only_is_decoded() {
        let dv = DynamicValue {
            msgpack: Vec::new(),
            json: b"[1,2]".to_vec(),
        };
        assert_eq!(to_json(&dv).unwrap(), json!([1, 2]));
    }

    #[test]
    fn empty_is_an_error() {
        let dv = DynamicValue {
            msgpack: Vec::new(),
            json: Vec::new(),
        };
        assert_eq!(
            to_json(&dv).unwrap_err().to_string(),
            "empty DynamicValue: neither msgpack nor json is set"
        );
    }

    #[test]
    fn unknown_value_is_refused_with_its_path() {
        // {"id": <unknown>} as cty encodes it: ext type 0, one zero byte.
        let mp = Mp::Map(vec![(Mp::String("id".into()), Mp::Ext(0, vec![0]))]);
        let mut msgpack = Vec::new();
        rmpv::encode::write_value(&mut msgpack, &mp).unwrap();
        let dv = DynamicValue {
            msgpack,
            json: Vec::new(),
        };
        assert_eq!(
            to_json(&dv).unwrap_err().to_string(),
            "id: provider returned an unknown value; unknowns only exist during planning and \
             cannot appear in an applied state"
        );
    }

    #[test]
    fn non_string_map_key_is_refused() {
        let mp = Mp::Map(vec![(Mp::Integer(1.into()), Mp::Nil)]);
        let mut msgpack = Vec::new();
        rmpv::encode::write_value(&mut msgpack, &mp).unwrap();
        let dv = DynamicValue {
            msgpack,
            json: Vec::new(),
        };
        assert_eq!(
            to_json(&dv).unwrap_err().to_string(),
            "<root>: msgpack map key is not a string"
        );
    }
}
