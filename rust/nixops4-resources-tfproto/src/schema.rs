//! Provider schema in the JSON shape that `tfCommonSchemaToModule`
//! consumes from Nix.
//!
//! Attribute types are carried verbatim as their go-cty JSON encoding —
//! the same bytes tfplugin puts in the `Attribute.type` field. The
//! adapter does not interpret cty; the Nix side matches literal
//! encodings (`"\"string\""` for a `string`, `"[\"list\",\"string\"]"`
//! for a `list(string)`, and so on).
//!
//! References:
//! - cty type system: <https://github.com/zclconf/go-cty>
//! - go-cty JSON type marshalling (`MarshalType` / `UnmarshalType`):
//!   <https://github.com/zclconf/go-cty/blob/main/cty/json/marshal.go>
//! - tfplugin `Attribute.type` field: `vendor/proto/tfplugin6.proto`

use anyhow::{Context, Result};
use serde::Serialize;
use std::collections::BTreeMap;

use crate::tfplugin6;

/// Provider schema in a stable JSON shape.
#[derive(Debug, Serialize)]
pub struct ProviderSchema {
    /// Managed resource types, keyed by type name.
    pub resource_schemas: BTreeMap<String, ResourceSchema>,
    /// Data source types, keyed by type name.
    pub data_source_schemas: BTreeMap<String, ResourceSchema>,
}

#[derive(Debug, Serialize)]
pub struct ResourceSchema {
    pub block: Block,
}

#[derive(Debug, Serialize)]
pub struct Block {
    pub attributes: BTreeMap<String, Attribute>,
    /// Legacy-style nested blocks (as opposed to cty nested attributes).
    /// Empty for most modern schemas but present in many real providers.
    #[serde(skip_serializing_if = "BTreeMap::is_empty")]
    pub block_types: BTreeMap<String, BlockType>,
    /// tfplugin `deprecated`: the whole block should not be used.
    /// Adapter emits a `provider warning:` per hit when the user's
    /// input touches it.
    #[serde(skip_serializing_if = "is_false", default)]
    pub deprecated: bool,
}

#[derive(Debug, Serialize)]
pub struct BlockType {
    pub nesting: NestingMode,
    /// `min_items = 0` marks a block whose presence is optional.
    /// `min_items >= 1` on a Single-nesting block turns it into a
    /// required block; on a List-nesting block it requires that many
    /// entries.
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub min_items: i64,
    /// `max_items = 0` means unbounded. Otherwise, an upper cap on the
    /// number of block instances. Forwarded verbatim; the Nix side
    /// does not yet enforce it.
    #[serde(skip_serializing_if = "is_zero_i64")]
    pub max_items: i64,
    pub block: Block,
}

fn is_zero_i64(n: &i64) -> bool {
    *n == 0
}

/// How multiple instances of a block relate in the config. `Single`
/// and `List` are the common ones; the others are supported by
/// tfplugin but not by this adapter yet.
#[derive(Debug, Serialize, PartialEq, Eq)]
pub enum NestingMode {
    Single,
    List,
    /// Multiple instances, treated as an unordered set. Order and
    /// duplicates are semantically noise; the provider deduplicates.
    Set,
    /// Multiple instances keyed by a string label (HCL: `role "admin"`).
    /// Maps to `types.attrsOf submodule` on the Nix side.
    Map,
    /// Any nesting mode we don't handle yet — the raw variant name from
    /// the proto is preserved so the Nix side can surface a specific
    /// error rather than a generic "unknown".
    Unsupported(String),
}

#[derive(Debug, Serialize)]
pub struct Attribute {
    /// go-cty JSON encoding of the attribute type, forwarded verbatim
    /// from the tfplugin `Attribute.type` bytes.
    #[serde(rename = "type")]
    pub cty_type: CtyType,
    pub required: bool,
    pub optional: bool,
    pub computed: bool,
    /// tfplugin `write_only`: user supplies this attribute each run
    /// (typically a secret or one-time token) and it must not be
    /// persisted in state. Adapter strips it from `prior_state`.
    #[serde(skip_serializing_if = "is_false", default)]
    pub write_only: bool,
    /// tfplugin `sensitive`: the attribute's value must not be
    /// surfaced in logs or display. The Nix side extends the option
    /// type with `nixops4Lib.types.sensitiveOf` so plain values are
    /// refused; the adapter wraps the corresponding output field as
    /// a nixops4 sensitive marker before returning it to nixops4.
    #[serde(skip_serializing_if = "is_false", default)]
    pub sensitive: bool,
    /// tfplugin `deprecated`: the attribute should not be used.
    /// Adapter emits a `provider warning:` per hit when the user's
    /// input touches it.
    #[serde(skip_serializing_if = "is_false", default)]
    pub deprecated: bool,
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

fn is_false(b: &bool) -> bool {
    !*b
}

/// go-cty JSON encoding of a type: a JSON string for primitives (`"string"`),
/// a JSON array for composites (`["list","string"]`). Held as UTF-8 text
/// because tfplugin transports it as `bytes` and go-cty JSON is JSON, which
/// is required to be UTF-8.
pub type CtyType = String;

impl ProviderSchema {
    pub fn from_proto(resp: tfplugin6::get_provider_schema::Response) -> Result<Self> {
        Ok(ProviderSchema {
            resource_schemas: resp
                .resource_schemas
                .into_iter()
                .map(|(name, schema)| {
                    ResourceSchema::from_proto(&name, schema).map(|rs| (name, rs))
                })
                .collect::<Result<_>>()?,
            data_source_schemas: resp
                .data_source_schemas
                .into_iter()
                .map(|(name, schema)| {
                    ResourceSchema::from_proto(&name, schema).map(|rs| (name, rs))
                })
                .collect::<Result<_>>()?,
        })
    }
}

impl ResourceSchema {
    fn from_proto(type_name: &str, schema: tfplugin6::Schema) -> Result<Self> {
        let block = match schema.block {
            Some(b) => Block::from_proto(type_name, b)?,
            None => Block::empty(),
        };
        Ok(ResourceSchema { block })
    }
}

impl Block {
    fn empty() -> Self {
        Block {
            attributes: BTreeMap::new(),
            block_types: BTreeMap::new(),
            deprecated: false,
        }
    }

    fn from_proto(type_name: &str, block: tfplugin6::schema::Block) -> Result<Self> {
        let attributes = block
            .attributes
            .into_iter()
            .map(|a| {
                let name = a.name.clone();
                Attribute::from_proto(type_name, a).map(|attr| (name, attr))
            })
            .collect::<Result<_>>()?;
        let block_types = block
            .block_types
            .into_iter()
            .map(|nb| {
                let name = nb.type_name.clone();
                BlockType::from_proto(type_name, nb).map(|bt| (name, bt))
            })
            .collect::<Result<_>>()?;
        Ok(Block {
            attributes,
            block_types,
            deprecated: block.deprecated,
        })
    }
}

impl BlockType {
    fn from_proto(parent: &str, nb: tfplugin6::schema::NestedBlock) -> Result<Self> {
        let nesting = NestingMode::from_proto(nb.nesting);
        let inner_block = nb
            .block
            .with_context(|| format!("block {}.{} has no inner block", parent, nb.type_name))?;
        let block = Block::from_proto(&format!("{}.{}", parent, nb.type_name), inner_block)?;
        Ok(BlockType {
            nesting,
            min_items: nb.min_items,
            max_items: nb.max_items,
            block,
        })
    }
}

impl NestingMode {
    fn from_proto(nesting: i32) -> Self {
        use tfplugin6::schema::nested_block::NestingMode as W;
        match W::try_from(nesting) {
            Ok(W::Single) => NestingMode::Single,
            Ok(W::List) => NestingMode::List,
            Ok(W::Set) => NestingMode::Set,
            Ok(W::Map) => NestingMode::Map,
            Ok(other) => NestingMode::Unsupported(format!("{:?}", other)),
            Err(_) => NestingMode::Unsupported(format!("unknown({})", nesting)),
        }
    }
}

impl Attribute {
    fn from_proto(type_name: &str, a: tfplugin6::schema::Attribute) -> Result<Self> {
        // go-cty JSON is JSON, which the spec requires to be UTF-8. A
        // provider that violates that is broken; surface immediately.
        let cty_type = String::from_utf8(a.r#type).with_context(|| {
            format!(
                "attribute {}.{} has non-UTF-8 go-cty JSON type",
                type_name, a.name
            )
        })?;
        Ok(Attribute {
            cty_type,
            required: a.required,
            optional: a.optional,
            computed: a.computed,
            write_only: a.write_only,
            sensitive: a.sensitive,
            deprecated: a.deprecated,
            description: a.description,
        })
    }
}
