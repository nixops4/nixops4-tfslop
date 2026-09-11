//! Collect deprecation-warning messages for the fields a user's
//! input actually touches.
//!
//! The tfplugin schema flags attributes and blocks as `deprecated`.
//! Emitting a warning for *every* deprecated field in a large schema
//! is noise — only fields the user set are actionable. This module
//! walks the schema with the input map in lockstep and returns one
//! message per hit.

use serde_json::{Map, Value};

use crate::schema::{Block, BlockType, NestingMode, ProviderSchema};

/// Format a deprecation warning for a single attribute or block path.
fn message(kind: &str, path: &str) -> String {
    format!("provider warning: {} {} is deprecated", kind, path)
}

/// Return one warning per deprecated attribute or block the input
/// map touches. Path segments follow the same dot / bracket notation
/// as the projection errors (`fx_thing.settings.rules[0].name`).
pub fn collect_deprecated_hits(
    schema: &ProviderSchema,
    type_name: &str,
    inputs: &Map<String, Value>,
) -> Vec<String> {
    let Some(resource) = schema.resource_schemas.get(type_name) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    walk_block(&resource.block, inputs, type_name, &mut out);
    out
}

fn walk_block(block: &Block, state: &Map<String, Value>, path: &str, out: &mut Vec<String>) {
    for (name, attr) in &block.attributes {
        if !state.contains_key(name) {
            continue;
        }
        if attr.deprecated {
            out.push(message("attribute", &format!("{}.{}", path, name)));
        }
    }
    for (name, bt) in &block.block_types {
        let Some(v) = state.get(name) else {
            continue;
        };
        if bt.block.deprecated {
            out.push(message("block", &format!("{}.{}", path, name)));
        }
        walk_block_type(bt, v, &format!("{}.{}", path, name), out);
    }
}

fn walk_block_type(bt: &BlockType, state: &Value, path: &str, out: &mut Vec<String>) {
    match &bt.nesting {
        NestingMode::Single => {
            if let Value::Object(m) = state {
                walk_block(&bt.block, m, path, out);
            }
        }
        NestingMode::List | NestingMode::Set => {
            if let Value::Array(arr) = state {
                for (i, elem) in arr.iter().enumerate() {
                    if let Value::Object(m) = elem {
                        walk_block(&bt.block, m, &format!("{}[{}]", path, i), out);
                    }
                }
            }
        }
        NestingMode::Map => {
            if let Value::Object(m) = state {
                for (key, elem) in m {
                    if let Value::Object(inner) = elem {
                        walk_block(&bt.block, inner, &format!("{}[\"{}\"]", path, key), out);
                    }
                }
            }
        }
        NestingMode::Unsupported(_) => {}
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Attribute, ResourceSchema};
    use serde_json::json;
    use std::collections::BTreeMap;

    fn attr(deprecated: bool) -> Attribute {
        Attribute {
            cty_type: "\"string\"".to_string(),
            required: false,
            optional: true,
            computed: false,
            write_only: false,
            sensitive: false,
            deprecated,
            description: String::new(),
        }
    }

    fn schema_with(type_name: &str, block: Block) -> ProviderSchema {
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

    #[test]
    fn no_hits_when_no_deprecations() {
        let mut attrs = BTreeMap::new();
        attrs.insert("value".to_string(), attr(false));
        let schema = schema_with(
            "fx_thing",
            Block {
                attributes: attrs,
                block_types: BTreeMap::new(),
                deprecated: false,
            },
        );
        let inputs = json!({"value": "hello"}).as_object().cloned().unwrap();
        assert!(collect_deprecated_hits(&schema, "fx_thing", &inputs).is_empty());
    }

    // The user set a deprecated attribute — one hit naming the path.
    #[test]
    fn hit_when_user_sets_deprecated_attribute() {
        let mut attrs = BTreeMap::new();
        attrs.insert("old_field".to_string(), attr(true));
        attrs.insert("new_field".to_string(), attr(false));
        let schema = schema_with(
            "fx_thing",
            Block {
                attributes: attrs,
                block_types: BTreeMap::new(),
                deprecated: false,
            },
        );
        let inputs = json!({"old_field": "still using it"})
            .as_object()
            .cloned()
            .unwrap();
        let hits = collect_deprecated_hits(&schema, "fx_thing", &inputs);
        assert_eq!(hits.len(), 1, "expected one hit; got {hits:?}");
        assert!(
            hits[0].contains("fx_thing.old_field"),
            "hit should name the path; got: {}",
            hits[0]
        );
    }

    // The user *didn't* set a deprecated attribute — no warning.
    // Emitting one anyway would be noise: any moderately-sized
    // schema has a long tail of deprecated fields that don't apply
    // to a given config.
    #[test]
    fn no_hit_when_user_does_not_set_deprecated_attribute() {
        let mut attrs = BTreeMap::new();
        attrs.insert("old_field".to_string(), attr(true));
        let schema = schema_with(
            "fx_thing",
            Block {
                attributes: attrs,
                block_types: BTreeMap::new(),
                deprecated: false,
            },
        );
        let inputs = Map::new();
        assert!(collect_deprecated_hits(&schema, "fx_thing", &inputs).is_empty());
    }

    // A deprecated block gets one hit for the block itself when the
    // user sets it (regardless of whether they set inner attributes).
    #[test]
    fn hit_when_user_sets_deprecated_block() {
        let inner_block = Block {
            attributes: BTreeMap::new(),
            block_types: BTreeMap::new(),
            deprecated: true,
        };
        let mut block_types = BTreeMap::new();
        block_types.insert(
            "settings".to_string(),
            BlockType {
                nesting: NestingMode::Single,
                min_items: 0,
                max_items: 0,
                block: inner_block,
            },
        );
        let schema = schema_with(
            "fx_thing",
            Block {
                attributes: BTreeMap::new(),
                block_types,
                deprecated: false,
            },
        );
        let inputs = json!({"settings": {}}).as_object().cloned().unwrap();
        let hits = collect_deprecated_hits(&schema, "fx_thing", &inputs);
        assert_eq!(hits.len(), 1, "expected one hit; got {hits:?}");
        assert!(
            hits[0].contains("fx_thing.settings"),
            "hit should name the path; got: {}",
            hits[0]
        );
    }
}
