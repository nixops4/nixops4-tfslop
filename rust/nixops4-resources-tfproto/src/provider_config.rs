//! Provider configuration travels inside the resource inputs, under the
//! reserved attribute `provider`.
//!
//! nixops4 has no provider-level configuration channel: a resource's
//! inputs are the only typed, sensitive-aware data that reaches the
//! adapter. Terraform reserves `provider` as a meta-argument on every
//! resource, so no resource schema can have an attribute of that name,
//! and the adapter can take it out of the inputs without ambiguity.
//! The Nix side declares `inputs.provider` on every resource type,
//! typed from the provider's own schema block.
//!
//! The value is the go-cty object that `ConfigureProvider` receives,
//! completed against the provider schema by the caller. On destroy it
//! comes from the stored inputs, so a resource is torn down with the
//! configuration it was created with.

use anyhow::{bail, Result};
use serde_json::{Map, Value};

pub const KEY: &str = "provider";

/// Take `provider` out of the inputs. Absent or null means "no
/// configuration", an empty object; anything but an object is refused.
pub fn split(mut inputs: Map<String, Value>) -> Result<(Map<String, Value>, Value)> {
    let config = match inputs.remove(KEY) {
        None | Some(Value::Null) => Value::Object(Map::new()),
        Some(v @ Value::Object(_)) => v,
        Some(other) => bail!(
            "inputs.provider must be an object of provider configuration, got {}",
            kind(&other)
        ),
    };
    Ok((inputs, config))
}

fn kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "list",
        Value::Object(_) => "object",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn map(v: Value) -> Map<String, Value> {
        match v {
            Value::Object(m) => m,
            _ => unreachable!(),
        }
    }

    #[test]
    fn provider_is_taken_out_of_the_inputs() {
        let (inputs, config) =
            split(map(json!({"name": "k", "provider": {"zone": "nl-ams-1"}}))).unwrap();
        assert_eq!(Value::Object(inputs), json!({"name": "k"}));
        assert_eq!(config, json!({"zone": "nl-ams-1"}));
    }

    #[test]
    fn absent_and_null_mean_empty_configuration() {
        let (_, absent) = split(map(json!({"name": "k"}))).unwrap();
        let (_, null) = split(map(json!({"name": "k", "provider": null}))).unwrap();
        assert_eq!(absent, json!({}));
        assert_eq!(null, json!({}));
    }

    #[test]
    fn non_object_is_refused() {
        assert_eq!(
            split(map(json!({"provider": "nl-ams-1"})))
                .unwrap_err()
                .to_string(),
            "inputs.provider must be an object of provider configuration, got string"
        );
    }
}
