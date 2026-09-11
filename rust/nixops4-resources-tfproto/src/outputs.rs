//! Project a tfplugin `new_state` (go-cty JSON) onto only the
//! attributes and blocks the schema flags as `computed`.
//!
//! The protocol explicitly discourages echoing user-supplied inputs
//! back as outputs. Structurally the projection mirrors the input
//! walk: attributes with `computed = true` become top-level output
//! entries, and blocks that recursively contain computed content
//! become nested output entries.

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

use crate::schema::{Block, BlockType, NestingMode, ProviderSchema};

/// Project the fields the schema marks as computed out of a resource's
/// `new_state`. Returns an empty map when there is nothing to project
/// (destroy path: `new_state == null`).
pub fn project_new_state(
    schema: &ProviderSchema,
    type_name: &str,
    new_state_json: &[u8],
) -> Result<Map<String, Value>> {
    let resource = schema
        .resource_schemas
        .get(type_name)
        .with_context(|| format!("no schema entry for resource type {}", type_name))?;
    let state: Value = serde_json::from_slice(new_state_json)
        .with_context(|| format!("parsing new_state for {} as JSON", type_name))?;
    let mut out = Map::new();
    // The path starts at the resource type name so deep failures
    // read as `fx_thing.settings.token: …` — the same dotted form a
    // user sees in their Nix config.
    project_block(&resource.block, &state, &mut out, type_name)?;
    Ok(out)
}

/// Human-readable name for a JSON value's shape. Used in error
/// messages instead of `{:?}` which prints Rust Debug syntax
/// (`String("hello")`, `Number(42)`) and leaks the internal type.
fn json_kind(v: &Value) -> &'static str {
    match v {
        Value::Null => "null",
        Value::Bool(_) => "boolean",
        Value::Number(_) => "number",
        Value::String(_) => "string",
        Value::Array(_) => "array",
        Value::Object(_) => "object",
    }
}

fn project_block(
    block: &Block,
    state: &Value,
    out: &mut Map<String, Value>,
    path: &str,
) -> Result<()> {
    let state_obj = match state {
        Value::Object(m) => m,
        // A null block instance carries no computed values.
        Value::Null => return Ok(()),
        other => bail!(
            "{}: expected object or null in new_state, got {}",
            path,
            json_kind(other)
        ),
    };
    for (name, attr) in &block.attributes {
        if !attr.computed {
            continue;
        }
        if let Some(v) = state_obj.get(name) {
            out.insert(name.clone(), v.clone());
        }
    }
    for (name, bt) in &block.block_types {
        if !block_has_computed(&bt.block) {
            continue;
        }
        let child_path = format!("{}.{}", path, name);
        let projected = project_block_type(bt, state_obj.get(name), &child_path)?;
        out.insert(name.clone(), projected);
    }
    Ok(())
}

fn project_block_type(bt: &BlockType, state: Option<&Value>, path: &str) -> Result<Value> {
    match &bt.nesting {
        NestingMode::Single => match state {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(v) => {
                let mut inner = Map::new();
                project_block(&bt.block, v, &mut inner, path)?;
                Ok(Value::Object(inner))
            }
        },
        NestingMode::List | NestingMode::Set => {
            let arr = state
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut items = Vec::with_capacity(arr.len());
            for (i, elem) in arr.iter().enumerate() {
                let mut inner = Map::new();
                let elem_path = format!("{}[{}]", path, i);
                project_block(&bt.block, elem, &mut inner, &elem_path)?;
                items.push(Value::Object(inner));
            }
            Ok(Value::Array(items))
        }
        NestingMode::Map => {
            let obj = state
                .and_then(|v| v.as_object())
                .cloned()
                .unwrap_or_default();
            let mut projected: Map<String, Value> = Map::new();
            for (key, elem) in obj.iter() {
                let mut inner = Map::new();
                let elem_path = format!("{}[\"{}\"]", path, key);
                project_block(&bt.block, elem, &mut inner, &elem_path)?;
                projected.insert(key.clone(), Value::Object(inner));
            }
            Ok(Value::Object(projected))
        }
        NestingMode::Unsupported(mode) => {
            bail!(
                "{}: cannot project new_state through unsupported nesting mode {}",
                path,
                mode
            )
        }
    }
}

pub(crate) fn block_has_computed(block: &Block) -> bool {
    block.attributes.values().any(|a| a.computed)
        || block
            .block_types
            .values()
            .any(|bt| block_has_computed(&bt.block))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Attribute, ResourceSchema};
    use std::collections::BTreeMap;

    fn attr(cty_type: &str, computed: bool) -> Attribute {
        Attribute {
            cty_type: cty_type.to_string(),
            required: false,
            optional: false,
            computed,
            write_only: false,
            sensitive: false,
            deprecated: false,
            description: String::new(),
        }
    }

    fn block_with_settings_containing_token() -> Block {
        let mut inner_attrs = BTreeMap::new();
        inner_attrs.insert("token".to_string(), attr("\"string\"", true));
        let inner = Block {
            attributes: inner_attrs,
            block_types: BTreeMap::new(),
            deprecated: false,
        };
        let mut block_types = BTreeMap::new();
        block_types.insert(
            "settings".to_string(),
            BlockType {
                nesting: NestingMode::Single,
                min_items: 0,
                max_items: 0,
                block: inner,
            },
        );
        Block {
            attributes: BTreeMap::new(),
            block_types,
            deprecated: false,
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

    // #74: projection errors must point at the attribute path they
    // failed on. The walker knows the path; not surfacing it makes
    // deep failures unactionable ("expected object, got Number" —
    // *where* though?).
    #[test]
    fn error_names_the_failing_path() {
        let schema = schema_with("fx_thing", block_with_settings_containing_token());
        // `settings` should be an object, but we feed a number instead.
        let new_state = br#"{"settings": 42}"#;
        let err = project_new_state(&schema, "fx_thing", new_state).unwrap_err();
        let chain = format!("{err:#}");
        assert!(
            chain.contains("fx_thing.settings"),
            "error should name the failing path fx_thing.settings; got: {chain}"
        );
    }

    // #76: user-facing errors must not use Rust's Debug format for
    // serde_json values. `Number(42)` and `String("hello")` leak the
    // Rust representation into the error message.
    #[test]
    fn error_does_not_use_debug_format_for_json_values() {
        let schema = schema_with("fx_thing", block_with_settings_containing_token());
        let new_state = br#"{"settings": 42}"#;
        let err = project_new_state(&schema, "fx_thing", new_state).unwrap_err();
        let chain = format!("{err:#}");
        for rust_debug_marker in ["Number(", "String(", "Bool(", "Array(", "Object("] {
            assert!(
                !chain.contains(rust_debug_marker),
                "found Rust Debug marker {rust_debug_marker:?} in user-facing error: {chain}"
            );
        }
    }

    // Sanity: a well-formed input still projects cleanly.
    #[test]
    fn happy_path_still_projects() {
        let schema = schema_with("fx_thing", block_with_settings_containing_token());
        let new_state = br#"{"settings": {"token": "abc"}}"#;
        let out = project_new_state(&schema, "fx_thing", new_state).unwrap();
        assert_eq!(
            out.get("settings")
                .and_then(|v| v.as_object())
                .and_then(|m| m.get("token"))
                .and_then(|v| v.as_str()),
            Some("abc")
        );
    }
}
