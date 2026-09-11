//! `proposed_new_state` for an update.
//!
//! Terraform does not send the bare config as the proposed state. It
//! merges the prior state in first (`objchange.ProposedNew`): a
//! computed attribute the config leaves null keeps its prior value,
//! so the provider sees "unchanged" rather than "unknown, recompute".
//! Without this, terraform-plugin-sdk v2 marks every such attribute
//! unknown on every update and re-derives it at apply time; correct,
//! but more work and noisier plans than necessary.
//!
//! Nested blocks are merged positionally for List nesting when both
//! sides have the same length, per key for Map nesting, and left as
//! the config otherwise. Set-nested blocks are left as the config:
//! Terraform matches their elements by value, which is more than this
//! needs to get right today.

use serde_json::{Map, Value};

use crate::schema::{Block, NestingMode};

pub fn proposed_new(block: &Block, prior: &Value, config: &Value) -> Value {
    let (Value::Object(prior), Value::Object(config)) = (prior, config) else {
        return config.clone();
    };
    let mut out = Map::new();
    for (name, attr) in &block.attributes {
        let from_config = config.get(name).cloned().unwrap_or(Value::Null);
        let v = if attr.computed && from_config.is_null() {
            prior.get(name).cloned().unwrap_or(Value::Null)
        } else {
            from_config
        };
        out.insert(name.clone(), v);
    }
    for (name, bt) in &block.block_types {
        let from_config = config.get(name).cloned().unwrap_or(Value::Null);
        let from_prior = prior.get(name);
        let v = match (&bt.nesting, from_prior, from_config) {
            (NestingMode::Single, Some(p), c) => proposed_new(&bt.block, p, &c),
            (NestingMode::List, Some(Value::Array(p)), Value::Array(c)) if p.len() == c.len() => {
                Value::Array(
                    p.iter()
                        .zip(c.iter())
                        .map(|(pe, ce)| proposed_new(&bt.block, pe, ce))
                        .collect(),
                )
            }
            (NestingMode::Map, Some(Value::Object(p)), Value::Object(c)) => Value::Object(
                c.into_iter()
                    .map(|(k, ce)| {
                        let merged = match p.get(&k) {
                            Some(pe) => proposed_new(&bt.block, pe, &ce),
                            None => ce,
                        };
                        (k, merged)
                    })
                    .collect(),
            ),
            (_, _, c) => c,
        };
        out.insert(name.clone(), v);
    }
    // Anything the schema does not know is left to `complete` to
    // refuse; carry it through unchanged so its message names it.
    for (k, v) in config {
        out.entry(k.clone()).or_insert_with(|| v.clone());
    }
    Value::Object(out)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::schema::{Attribute, BlockType};
    use serde_json::json;

    fn attr(computed: bool) -> Attribute {
        Attribute {
            cty_type: "\"string\"".to_string(),
            required: !computed,
            optional: computed,
            computed,
            write_only: false,
            sensitive: false,
            deprecated: false,
            description: String::new(),
        }
    }

    fn block(attrs: Vec<(&str, bool)>, blocks: Vec<(&str, NestingMode, Block)>) -> Block {
        Block {
            attributes: attrs
                .into_iter()
                .map(|(k, c)| (k.to_string(), attr(c)))
                .collect(),
            block_types: blocks
                .into_iter()
                .map(|(k, nesting, inner)| {
                    (
                        k.to_string(),
                        BlockType {
                            nesting,
                            min_items: 0,
                            max_items: 0,
                            block: inner,
                        },
                    )
                })
                .collect(),
            deprecated: false,
        }
    }

    #[test]
    fn computed_null_in_config_takes_prior_value() {
        let b = block(vec![("name", false), ("id", true), ("zone", true)], vec![]);
        let prior = json!({"name": "old", "id": "i-1", "zone": "nl-ams-1"});
        let config = json!({"name": "new", "id": null, "zone": "fr-par-1"});
        assert_eq!(
            proposed_new(&b, &prior, &config),
            json!({"name": "new", "id": "i-1", "zone": "fr-par-1"})
        );
    }

    #[test]
    fn non_computed_null_stays_null() {
        let b = block(vec![("name", false)], vec![]);
        assert_eq!(
            proposed_new(&b, &json!({"name": "old"}), &json!({"name": null})),
            json!({"name": null})
        );
    }

    #[test]
    fn create_has_no_prior_and_returns_config() {
        let b = block(vec![("id", true)], vec![]);
        assert_eq!(
            proposed_new(&b, &Value::Null, &json!({"id": null})),
            json!({"id": null})
        );
    }

    #[test]
    fn list_blocks_merge_positionally_when_lengths_match() {
        let inner = block(vec![("size", false), ("volume_id", true)], vec![]);
        let b = block(vec![], vec![("root_volume", NestingMode::List, inner)]);
        let prior = json!({"root_volume": [{"size": 20, "volume_id": "v-1"}]});
        let config = json!({"root_volume": [{"size": 20, "volume_id": null}]});
        assert_eq!(
            proposed_new(&b, &prior, &config),
            json!({"root_volume": [{"size": 20, "volume_id": "v-1"}]})
        );
    }

    #[test]
    fn list_blocks_of_different_length_take_config() {
        let inner = block(vec![("size", false), ("volume_id", true)], vec![]);
        let b = block(vec![], vec![("root_volume", NestingMode::List, inner)]);
        let prior = json!({"root_volume": [{"size": 20, "volume_id": "v-1"}]});
        let config = json!({"root_volume": []});
        assert_eq!(
            proposed_new(&b, &prior, &config),
            json!({"root_volume": []})
        );
    }

    #[test]
    fn single_and_map_blocks_merge_inside() {
        let inner = block(vec![("x", false), ("c", true)], vec![]);
        let b = block(
            vec![],
            vec![
                ("single", NestingMode::Single, inner.clone()),
                ("map", NestingMode::Map, inner),
            ],
        );
        let prior = json!({"single": {"x": 1, "c": "p"}, "map": {"a": {"x": 1, "c": "pa"}}});
        let config = json!({"single": {"x": 2, "c": null}, "map": {"a": {"x": 1, "c": null}, "b": {"x": 3, "c": null}}});
        assert_eq!(
            proposed_new(&b, &prior, &config),
            json!({"single": {"x": 2, "c": "p"}, "map": {"a": {"x": 1, "c": "pa"}, "b": {"x": 3, "c": null}}})
        );
    }
}
