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

use crate::schema::{Block, BlockType, NestingMode, ProviderSchema};

/// Walk the projected outputs and, for every attribute the schema
/// declares sensitive, replace its value with a nixops4 sensitive
/// marker. Recurses into nested blocks so sensitive attributes
/// inside `Single` / `List` / `Set` / `Map` nestings are wrapped
/// per-instance, matching the shape `outputs::project_new_state`
/// produces.
pub fn wrap_sensitive_outputs(
    schema: &ProviderSchema,
    type_name: &str,
    mut outputs: Map<String, Value>,
) -> Result<Map<String, Value>> {
    let Some(resource) = schema.resource_schemas.get(type_name) else {
        // Unknown resource type — nothing to wrap. The projection
        // step (called just before us) already surfaces this as an
        // error where it matters.
        return Ok(outputs);
    };
    wrap_block(&resource.block, &mut outputs);
    Ok(outputs)
}

fn wrap_block(block: &Block, map: &mut Map<String, Value>) {
    for (name, attr) in &block.attributes {
        if !attr.sensitive {
            continue;
        }
        if let Some(v) = map.remove(name) {
            map.insert(name.clone(), mark(v));
        }
    }
    for (name, bt) in &block.block_types {
        if let Some(v) = map.remove(name) {
            map.insert(name.clone(), wrap_block_type(bt, v));
        }
    }
}

fn wrap_block_type(bt: &BlockType, value: Value) -> Value {
    // Shape mismatches (e.g. Single with a non-object) leave the
    // value untouched: the projection is authoritative about
    // shape, so if we got something odd here the bug is upstream
    // and there's nothing sensible to wrap.
    match &bt.nesting {
        NestingMode::Single => wrap_instance(&bt.block, value),
        NestingMode::List | NestingMode::Set => match value {
            Value::Array(arr) => Value::Array(
                arr.into_iter()
                    .map(|elem| wrap_instance(&bt.block, elem))
                    .collect(),
            ),
            other => other,
        },
        NestingMode::Map => match value {
            Value::Object(m) => Value::Object(
                m.into_iter()
                    .map(|(k, elem)| (k, wrap_instance(&bt.block, elem)))
                    .collect(),
            ),
            other => other,
        },
        // Projection would have already bailed on Unsupported;
        // defensive no-op keeps this function total.
        NestingMode::Unsupported(_) => value,
    }
}

fn wrap_instance(block: &Block, value: Value) -> Value {
    match value {
        Value::Object(mut m) => {
            wrap_block(block, &mut m);
            Value::Object(m)
        }
        other => other,
    }
}

/// Construct the marker shape around `inner`. Provenances stay off
/// the output — nixops4-eval's `fromSerializable` synthesises a
/// position-derived provenance on ingress.
fn mark(inner: Value) -> Value {
    let mut m = Map::new();
    m.insert("_type".to_string(), Value::String("sensitive".to_string()));
    m.insert("value".to_string(), inner);
    Value::Object(m)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Attribute, Block, BlockType, NestingMode, ResourceSchema};
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

    fn block_with(attrs: Vec<(&str, Attribute)>, block_types: Vec<(&str, BlockType)>) -> Block {
        let mut attributes = BTreeMap::new();
        for (name, a) in attrs {
            attributes.insert(name.to_string(), a);
        }
        let mut bts = BTreeMap::new();
        for (name, bt) in block_types {
            bts.insert(name.to_string(), bt);
        }
        Block {
            attributes,
            block_types: bts,
            deprecated: false,
        }
    }

    fn nested(nesting: NestingMode, block: Block) -> BlockType {
        BlockType {
            nesting,
            min_items: 0,
            max_items: 0,
            block,
        }
    }

    fn schema_with_block(type_name: &str, block: Block) -> ProviderSchema {
        let mut rs = BTreeMap::new();
        rs.insert(type_name.to_string(), ResourceSchema { block });
        ProviderSchema {
            provider: crate::schema::ResourceSchema {
                block: crate::schema::Block::empty(),
            },
            resource_schemas: rs,
            data_source_schemas: BTreeMap::new(),
        }
    }

    fn schema_with(type_name: &str, attrs: Vec<(&str, Attribute)>) -> ProviderSchema {
        schema_with_block(type_name, block_with(attrs, vec![]))
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

    // The next four tests exercise nested-block sensitive attributes,
    // one per NestingMode. `project_new_state` already produces the
    // nested shape (Object / Array / Map of Object) and we need to
    // walk into the same shape when wrapping sensitives.

    #[test]
    fn wraps_sensitive_attribute_inside_single_nested_block() {
        let schema = schema_with_block(
            "fx_thing",
            block_with(
                vec![],
                vec![(
                    "settings",
                    nested(
                        NestingMode::Single,
                        block_with(vec![("token", attr(true, true))], vec![]),
                    ),
                )],
            ),
        );
        let inputs = json!({"settings": {"token": "s3cret"}})
            .as_object()
            .unwrap()
            .clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        assert_eq!(
            got.get("settings")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("token")),
            Some(&json!({"_type": "sensitive", "value": "s3cret"}))
        );
    }

    #[test]
    fn wraps_sensitive_attribute_inside_list_nested_block() {
        let schema = schema_with_block(
            "fx_thing",
            block_with(
                vec![],
                vec![(
                    "entries",
                    nested(
                        NestingMode::List,
                        block_with(vec![("token", attr(true, true))], vec![]),
                    ),
                )],
            ),
        );
        let inputs = json!({"entries": [{"token": "a"}, {"token": "b"}]})
            .as_object()
            .unwrap()
            .clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        let arr = got.get("entries").and_then(|v| v.as_array()).unwrap();
        assert_eq!(
            arr[0].as_object().unwrap().get("token"),
            Some(&json!({"_type": "sensitive", "value": "a"}))
        );
        assert_eq!(
            arr[1].as_object().unwrap().get("token"),
            Some(&json!({"_type": "sensitive", "value": "b"}))
        );
    }

    #[test]
    fn wraps_sensitive_attribute_inside_set_nested_block() {
        // Set is projected as an unordered array of objects — same
        // per-element wrapping as List.
        let schema = schema_with_block(
            "fx_thing",
            block_with(
                vec![],
                vec![(
                    "entries",
                    nested(
                        NestingMode::Set,
                        block_with(vec![("token", attr(true, true))], vec![]),
                    ),
                )],
            ),
        );
        let inputs = json!({"entries": [{"token": "only"}]})
            .as_object()
            .unwrap()
            .clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        let arr = got.get("entries").and_then(|v| v.as_array()).unwrap();
        assert_eq!(
            arr[0].as_object().unwrap().get("token"),
            Some(&json!({"_type": "sensitive", "value": "only"}))
        );
    }

    #[test]
    fn wraps_sensitive_attribute_inside_map_nested_block() {
        let schema = schema_with_block(
            "fx_thing",
            block_with(
                vec![],
                vec![(
                    "entries",
                    nested(
                        NestingMode::Map,
                        block_with(vec![("token", attr(true, true))], vec![]),
                    ),
                )],
            ),
        );
        let inputs = json!({"entries": {"k1": {"token": "a"}, "k2": {"token": "b"}}})
            .as_object()
            .unwrap()
            .clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        let obj = got.get("entries").and_then(|v| v.as_object()).unwrap();
        assert_eq!(
            obj.get("k1").unwrap().as_object().unwrap().get("token"),
            Some(&json!({"_type": "sensitive", "value": "a"}))
        );
        assert_eq!(
            obj.get("k2").unwrap().as_object().unwrap().get("token"),
            Some(&json!({"_type": "sensitive", "value": "b"}))
        );
    }

    #[test]
    fn wraps_sensitive_attribute_in_deeply_nested_block() {
        // `settings { advanced { token } }` — recursion must not stop
        // at the first block boundary.
        let inner = nested(
            NestingMode::Single,
            block_with(vec![("token", attr(true, true))], vec![]),
        );
        let outer = nested(
            NestingMode::Single,
            block_with(vec![], vec![("advanced", inner)]),
        );
        let schema = schema_with_block("fx_thing", block_with(vec![], vec![("settings", outer)]));
        let inputs = json!({"settings": {"advanced": {"token": "deep"}}})
            .as_object()
            .unwrap()
            .clone();
        let got = wrap_sensitive_outputs(&schema, "fx_thing", inputs).unwrap();
        let deep = got
            .get("settings")
            .and_then(|v| v.as_object())
            .and_then(|m| m.get("advanced"))
            .and_then(|v| v.as_object())
            .and_then(|m| m.get("token"));
        assert_eq!(deep, Some(&json!({"_type": "sensitive", "value": "deep"})));
    }
}
