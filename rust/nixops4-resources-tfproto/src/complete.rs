//! Make a go-cty value total against its schema before it goes on the
//! wire.
//!
//! cty's msgpack decoder (`unmarshalObject`) requires an object to
//! carry exactly the attributes of its type: "an object with 9
//! attributes is required (5 given)". Its JSON decoder fills missing
//! attributes with null instead, which is why omissions never showed
//! up while the adapter spoke JSON only. Terraform core always sends
//! total objects, so providers are entitled to assume them.
//!
//! nixops4 hands the adapter only what the user set, plus null for
//! omitted optional inputs. This module adds the rest by walking the
//! schema: computed-only attributes become null, absent Single blocks
//! become null, absent List/Set blocks become empty lists, absent Map
//! blocks become empty maps, and cty object types inside attribute
//! types are completed field by field.

use anyhow::{bail, Context, Result};
use serde_json::{Map, Value};

use crate::schema::{Block, NestingMode};

/// Complete a block-shaped value. `Null` stays `Null` (a resource that
/// does not exist yet, or is being destroyed).
pub fn complete_block(block: &Block, value: Value, path: &str) -> Result<Value> {
    match value {
        Value::Null => Ok(Value::Null),
        Value::Object(fields) => Ok(Value::Object(complete_fields(block, fields, path)?)),
        other => bail!(
            "{}: expected an object for a block, got {}",
            path,
            kind(&other)
        ),
    }
}

fn complete_fields(
    block: &Block,
    mut fields: Map<String, Value>,
    path: &str,
) -> Result<Map<String, Value>> {
    let mut out = Map::new();
    for (name, attr) in &block.attributes {
        let child = format!("{}.{}", path, name);
        let ty: Value = serde_json::from_str(&attr.cty_type)
            .with_context(|| format!("{}: parsing cty type {}", child, attr.cty_type))?;
        let v = fields.remove(name).unwrap_or(Value::Null);
        out.insert(name.clone(), complete_by_cty(&ty, v, &child)?);
    }
    for (name, bt) in &block.block_types {
        let child = format!("{}.{}", path, name);
        let v = fields.remove(name).unwrap_or(Value::Null);
        let completed = match bt.nesting {
            NestingMode::Single => complete_block(&bt.block, v, &child)?,
            NestingMode::List | NestingMode::Set => match v {
                Value::Null => Value::Array(Vec::new()),
                Value::Array(items) => Value::Array(
                    items
                        .into_iter()
                        .enumerate()
                        .map(|(i, item)| {
                            complete_block(&bt.block, item, &format!("{}[{}]", child, i))
                        })
                        .collect::<Result<_>>()?,
                ),
                other => bail!("{}: expected a list of blocks, got {}", child, kind(&other)),
            },
            NestingMode::Map => match v {
                Value::Null => Value::Object(Map::new()),
                Value::Object(entries) => Value::Object(
                    entries
                        .into_iter()
                        .map(|(k, item)| {
                            complete_block(&bt.block, item, &format!("{}.{}", child, k))
                                .map(|c| (k, c))
                        })
                        .collect::<Result<_>>()?,
                ),
                other => bail!("{}: expected a map of blocks, got {}", child, kind(&other)),
            },
            NestingMode::Unsupported(ref mode) => {
                bail!("{}: unsupported block nesting mode {}", child, mode)
            }
        };
        out.insert(name.clone(), completed);
    }
    if let Some(extra) = fields.keys().next() {
        bail!(
            "{}.{}: not an attribute or block of this schema",
            path,
            extra
        );
    }
    Ok(out)
}

/// Complete a value against a go-cty type (the parsed JSON form:
/// `"string"`, `["list", T]`, `["object", {…}]`, ...). Only object
/// types need work; everything else is passed through, including
/// values whose shape does not match, which the provider will reject
/// with a more specific message than we could.
fn complete_by_cty(ty: &Value, value: Value, path: &str) -> Result<Value> {
    let (kind_name, arg) = match ty {
        Value::Array(parts) if parts.len() == 2 => match &parts[0] {
            Value::String(k) => (k.as_str(), &parts[1]),
            _ => return Ok(value),
        },
        _ => return Ok(value),
    };
    match (kind_name, value) {
        ("object", Value::Object(mut fields)) => {
            let Value::Object(field_types) = arg else {
                return Ok(Value::Object(fields));
            };
            let mut out = Map::new();
            for (name, fty) in field_types {
                let v = fields.remove(name).unwrap_or(Value::Null);
                out.insert(
                    name.clone(),
                    complete_by_cty(fty, v, &format!("{}.{}", path, name))?,
                );
            }
            if let Some(extra) = fields.keys().next() {
                bail!("{}.{}: not a field of this object type", path, extra);
            }
            Ok(Value::Object(out))
        }
        ("list" | "set", Value::Array(items)) => Ok(Value::Array(
            items
                .into_iter()
                .enumerate()
                .map(|(i, item)| complete_by_cty(arg, item, &format!("{}[{}]", path, i)))
                .collect::<Result<_>>()?,
        )),
        ("map", Value::Object(entries)) => Ok(Value::Object(
            entries
                .into_iter()
                .map(|(k, item)| {
                    complete_by_cty(arg, item, &format!("{}.{}", path, k)).map(|c| (k, c))
                })
                .collect::<Result<_>>()?,
        )),
        ("tuple", Value::Array(items)) => {
            let Value::Array(elem_types) = arg else {
                return Ok(Value::Array(items));
            };
            if elem_types.len() != items.len() {
                return Ok(Value::Array(items));
            }
            Ok(Value::Array(
                items
                    .into_iter()
                    .zip(elem_types)
                    .enumerate()
                    .map(|(i, (item, ety))| complete_by_cty(ety, item, &format!("{}[{}]", path, i)))
                    .collect::<Result<_>>()?,
            ))
        }
        (_, value) => Ok(value),
    }
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
    use crate::schema::{Attribute, BlockType};
    use serde_json::json;

    fn attr(ty: &str, computed: bool) -> Attribute {
        Attribute {
            cty_type: ty.to_string(),
            required: !computed,
            optional: false,
            computed,
            write_only: false,
            sensitive: false,
            deprecated: false,
            description: String::new(),
        }
    }

    fn block(attrs: Vec<(&str, Attribute)>, blocks: Vec<(&str, BlockType)>) -> Block {
        Block {
            attributes: attrs.into_iter().map(|(k, v)| (k.to_string(), v)).collect(),
            block_types: blocks
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
            deprecated: false,
        }
    }

    fn nested(nesting: NestingMode, inner: Block) -> BlockType {
        BlockType {
            nesting,
            min_items: 0,
            max_items: 0,
            block: inner,
        }
    }

    fn ssh_key_like() -> Block {
        block(
            vec![
                ("name", attr("\"string\"", false)),
                ("public_key", attr("\"string\"", false)),
                ("id", attr("\"string\"", true)),
                ("fingerprint", attr("\"string\"", true)),
            ],
            vec![],
        )
    }

    #[test]
    fn computed_attributes_are_filled_with_null() {
        let got = complete_block(
            &ssh_key_like(),
            json!({"name": "k", "public_key": "ssh-ed25519 AAAA"}),
            "scaleway_iam_ssh_key",
        )
        .unwrap();
        assert_eq!(
            got,
            json!({"name": "k", "public_key": "ssh-ed25519 AAAA", "id": null, "fingerprint": null})
        );
    }

    #[test]
    fn null_resource_stays_null() {
        assert_eq!(
            complete_block(&ssh_key_like(), Value::Null, "t").unwrap(),
            Value::Null
        );
    }

    #[test]
    fn unknown_attribute_is_an_error() {
        assert_eq!(
            complete_block(&ssh_key_like(), json!({"provider": {}}), "t")
                .unwrap_err()
                .to_string(),
            "t.provider: not an attribute or block of this schema"
        );
    }

    #[test]
    fn absent_blocks_get_their_empty_value() {
        let inner = block(vec![("port", attr("\"number\"", false))], vec![]);
        let b = block(
            vec![],
            vec![
                ("single", nested(NestingMode::Single, inner.clone())),
                ("list", nested(NestingMode::List, inner.clone())),
                ("set", nested(NestingMode::Set, inner.clone())),
                ("map", nested(NestingMode::Map, inner)),
            ],
        );
        assert_eq!(
            complete_block(&b, json!({}), "t").unwrap(),
            json!({"single": null, "list": [], "set": [], "map": {}})
        );
    }

    #[test]
    fn present_blocks_are_completed_inside() {
        let inner = block(
            vec![
                ("port", attr("\"number\"", false)),
                ("id", attr("\"string\"", true)),
            ],
            vec![],
        );
        let b = block(
            vec![],
            vec![
                ("single", nested(NestingMode::Single, inner.clone())),
                ("list", nested(NestingMode::List, inner)),
            ],
        );
        assert_eq!(
            complete_block(
                &b,
                json!({"single": {"port": 22}, "list": [{"port": 80}]}),
                "t"
            )
            .unwrap(),
            json!({"single": {"port": 22, "id": null}, "list": [{"port": 80, "id": null}]})
        );
    }

    #[test]
    fn object_typed_attributes_are_completed_field_by_field() {
        let b = block(
            vec![
                (
                    "endpoints",
                    attr(
                        r#"["list",["object",{"ip":"string","port":"number"}]]"#,
                        false,
                    ),
                ),
                (
                    "meta",
                    attr(
                        r#"["object",{"owner":"string","tags":["map","string"]}]"#,
                        false,
                    ),
                ),
            ],
            vec![],
        );
        assert_eq!(
            complete_block(
                &b,
                json!({"endpoints": [{"ip": "10.0.0.1"}], "meta": {"tags": {"a": "b"}}}),
                "t"
            )
            .unwrap(),
            json!({
                "endpoints": [{"ip": "10.0.0.1", "port": null}],
                "meta": {"owner": null, "tags": {"a": "b"}}
            })
        );
    }

    #[test]
    fn unknown_object_field_is_an_error() {
        let b = block(
            vec![("meta", attr(r#"["object",{"owner":"string"}]"#, false))],
            vec![],
        );
        assert_eq!(
            complete_block(&b, json!({"meta": {"ownr": "x"}}), "t")
                .unwrap_err()
                .to_string(),
            "t.meta.ownr: not a field of this object type"
        );
    }
}
