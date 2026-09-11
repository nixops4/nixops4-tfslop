//! Reconstruct `prior_state` for a tfplugin update or destroy.
//!
//! nixops4 splits a resource's state into `input_properties` (what the
//! user asked for) and `output_properties` (what the provider computed
//! back). tfplugin's Plan+Apply want them merged into a single go-cty
//! JSON object.
//!
//! The merge is not a straight overlay: attributes marked
//! `write_only` on the schema must be stripped. Providers use this
//! flag for values the user supplies each run but that must never
//! land in state — secrets, one-time tokens. Feeding a stale
//! write-only value back would either poison the plan or, worse,
//! leak a secret the user meant to rotate.

use anyhow::{Context, Result};
use serde_json::{Map, Value};

use crate::schema::{Block, BlockType, NestingMode, ProviderSchema};

/// Build a full go-cty JSON prior_state from the nixops4 view, then
/// strip fields the schema marks as `write_only`.
pub fn build_prior_state_json(
    schema: &ProviderSchema,
    type_name: &str,
    prev_inputs: Map<String, Value>,
    prev_outputs: Option<Map<String, Value>>,
) -> Result<Vec<u8>> {
    let resource = schema
        .resource_schemas
        .get(type_name)
        .with_context(|| format!("no schema entry for resource type {}", type_name))?;

    // Merge outputs on top of inputs. Where a provider reused an
    // attribute name for a computed field, the later (output) value
    // wins — same order the provider itself would have produced.
    let mut prior = prev_inputs;
    if let Some(outputs) = prev_outputs {
        for (k, v) in outputs {
            prior.insert(k, v);
        }
    }

    strip_write_only_block(&resource.block, &mut prior);

    // Stored inputs and outputs together may still miss attributes
    // (write-only ones just stripped, computed ones the provider left
    // null and nixops4 did not store); cty msgpack wants them all.
    let prior = crate::complete::complete_block(&resource.block, Value::Object(prior), type_name)?;

    serde_json::to_vec(&prior).with_context(|| format!("serialising prior_state for {}", type_name))
}

fn strip_write_only_block(block: &Block, state: &mut Map<String, Value>) {
    for (name, attr) in &block.attributes {
        if attr.write_only {
            state.remove(name);
        }
    }
    for (name, bt) in &block.block_types {
        if !block_has_write_only(&bt.block) {
            continue;
        }
        if let Some(v) = state.get_mut(name) {
            strip_write_only_block_type(bt, v);
        }
    }
}

fn strip_write_only_block_type(bt: &BlockType, state: &mut Value) {
    match &bt.nesting {
        NestingMode::Single => {
            if let Value::Object(m) = state {
                strip_write_only_block(&bt.block, m);
            }
        }
        NestingMode::List | NestingMode::Set => {
            if let Value::Array(arr) = state {
                for elem in arr.iter_mut() {
                    if let Value::Object(m) = elem {
                        strip_write_only_block(&bt.block, m);
                    }
                }
            }
        }
        NestingMode::Map => {
            if let Value::Object(m) = state {
                for elem in m.values_mut() {
                    if let Value::Object(inner) = elem {
                        strip_write_only_block(&bt.block, inner);
                    }
                }
            }
        }
        NestingMode::Unsupported(_) => {
            // Nesting we can't translate on the way in either; leave
            // the state alone rather than mangle it silently.
        }
    }
}

fn block_has_write_only(block: &Block) -> bool {
    block.attributes.values().any(|a| a.write_only)
        || block
            .block_types
            .values()
            .any(|bt| block_has_write_only(&bt.block))
}
