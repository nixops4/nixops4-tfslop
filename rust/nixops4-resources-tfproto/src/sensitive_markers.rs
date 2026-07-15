//! Construct nixops4 sensitive markers around output values whose
//! schema declares them sensitive.
//!
//! tfplugin providers return plain JSON values on their outputs
//! (they don't speak the nixops4 marker convention). The adapter is
//! the layer that knows both about the tfplugin `sensitive` flag on
//! the schema and about the nixops4 marker shape. When the schema
//! declares a computed attribute sensitive, this module wraps the
//! projected value as `{"_type":"sensitive","value":X}` before
//! handing it up to nixops4.
//!
//! Result: nixops4 stores the marker in state, and downstream Nix
//! code (via nixops4-eval's `fromSerializable`) observes an eval-
//! time marker in slots the translator typed `sensitiveOf T`.

use anyhow::Result;
use serde_json::{Map, Value};

use crate::schema::ProviderSchema;

/// Walk the projected outputs and, for every top-level attribute
/// the schema declares sensitive, replace its value with a
/// nixops4 sensitive marker.
///
/// Nested-block sensitive attributes are out of scope for this
/// slice — task #97 tracks the extension.
pub fn wrap_sensitive_outputs(
    _schema: &ProviderSchema,
    _type_name: &str,
    _outputs: Map<String, Value>,
) -> Result<Map<String, Value>> {
    todo!("wrap_sensitive_outputs is unimplemented")
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Attribute, Block, ResourceSchema};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn attr(sensitive: bool, computed: bool) -> Attribute {
        Attribute {
            cty_type: "\"string\"".to_string(),
            required: false,
            optional: false,
            computed,
            write_only: false,
            sensitive,
            deprecated: false,
            description: String::new(),
        }
    }

    fn schema_with(type_name: &str, attrs: Vec<(&str, Attribute)>) -> ProviderSchema {
        let mut resource_attrs = BTreeMap::new();
        for (name, a) in attrs {
            resource_attrs.insert(name.to_string(), a);
        }
        let mut rs = BTreeMap::new();
        rs.insert(
            type_name.to_string(),
            ResourceSchema {
                block: Block {
                    attributes: resource_attrs,
                    block_types: BTreeMap::new(),
                    deprecated: false,
                },
            },
        );
        ProviderSchema {
            resource_schemas: rs,
            data_source_schemas: BTreeMap::new(),
        }
    }

    #[test]
    fn wraps_a_sensitive_output_attribute() {
        let schema = schema_with(
            "fx_thing",
            vec![("password", attr(/* sensitive */ true, /* computed */ true))],
        );
        let inputs = json!({"password": "s3cret"}).as_object().unwrap().clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        assert_eq!(
            got.get("password"),
            Some(&json!({"_type": "sensitive", "value": "s3cret"}))
        );
    }

    #[test]
    fn leaves_non_sensitive_output_alone() {
        let schema = schema_with(
            "fx_thing",
            vec![("value", attr(/* sensitive */ false, /* computed */ true))],
        );
        let inputs = json!({"value": "plain"}).as_object().unwrap().clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        assert_eq!(got.get("value"), Some(&json!("plain")));
    }

    #[test]
    fn passes_through_unknown_attributes_unchanged() {
        // The projection is authoritative about what's in the map;
        // the sensitive walker doesn't need to enforce schema
        // membership — it just wraps what the schema declares
        // sensitive. Unknown attributes stay as-is.
        let schema = schema_with(
            "fx_thing",
            vec![("password", attr(/* sensitive */ true, /* computed */ true))],
        );
        let inputs = json!({
            "password": "s3cret",
            "surprise": "extra",
        })
        .as_object()
        .unwrap()
        .clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        assert_eq!(
            got.get("password"),
            Some(&json!({"_type": "sensitive", "value": "s3cret"}))
        );
        assert_eq!(got.get("surprise"), Some(&json!("extra")));
    }

    #[test]
    fn handles_missing_attribute_gracefully() {
        // If the projection produced no value for a sensitive
        // attribute (the provider didn't emit it), we leave the
        // map unchanged rather than injecting a null-value marker.
        let schema = schema_with(
            "fx_thing",
            vec![("password", attr(/* sensitive */ true, /* computed */ true))],
        );
        let inputs = json!({}).as_object().unwrap().clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        assert!(!got.contains_key("password"));
    }
}
