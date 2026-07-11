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
    project_block(&resource.block, &state, &mut out)
        .with_context(|| format!("projecting new_state for {}", type_name))?;
    Ok(out)
}

fn project_block(block: &Block, state: &Value, out: &mut Map<String, Value>) -> Result<()> {
    let state_obj = match state {
        Value::Object(m) => m,
        // A null block instance carries no computed values.
        Value::Null => return Ok(()),
        other => bail!("expected object or null in new_state, got {:?}", other),
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
        let projected = project_block_type(bt, state_obj.get(name))?;
        out.insert(name.clone(), projected);
    }
    Ok(())
}

fn project_block_type(bt: &BlockType, state: Option<&Value>) -> Result<Value> {
    match &bt.nesting {
        NestingMode::Single => match state {
            None | Some(Value::Null) => Ok(Value::Null),
            Some(v) => {
                let mut inner = Map::new();
                project_block(&bt.block, v, &mut inner)?;
                Ok(Value::Object(inner))
            }
        },
        NestingMode::List => {
            let arr = state
                .and_then(|v| v.as_array())
                .cloned()
                .unwrap_or_default();
            let mut items = Vec::with_capacity(arr.len());
            for elem in &arr {
                let mut inner = Map::new();
                project_block(&bt.block, elem, &mut inner)?;
                items.push(Value::Object(inner));
            }
            Ok(Value::Array(items))
        }
        NestingMode::Unsupported(mode) => {
            bail!(
                "cannot project new_state through unsupported nesting mode {}",
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
