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
    #[serde(skip_serializing_if = "String::is_empty")]
    pub description: String,
}

/// go-cty JSON encoding of a type: a JSON string for primitives (`"string"`),
/// a JSON array for composites (`["list","string"]`). Held as UTF-8 text
/// because tfplugin transports it as `bytes` and go-cty JSON is JSON, which
/// is required to be UTF-8.
pub type CtyType = String;

impl ProviderSchema {
    pub fn from_wire(resp: tfplugin6::get_provider_schema::Response) -> Result<Self> {
        Ok(ProviderSchema {
            resource_schemas: resp
                .resource_schemas
                .into_iter()
                .map(|(name, schema)| ResourceSchema::from_wire(&name, schema).map(|rs| (name, rs)))
                .collect::<Result<_>>()?,
            data_source_schemas: resp
                .data_source_schemas
                .into_iter()
                .map(|(name, schema)| ResourceSchema::from_wire(&name, schema).map(|rs| (name, rs)))
                .collect::<Result<_>>()?,
        })
    }
}

impl ResourceSchema {
    fn from_wire(type_name: &str, schema: tfplugin6::Schema) -> Result<Self> {
        let block = match schema.block {
            Some(b) => Block::from_wire(type_name, b)?,
            None => Block::empty(),
        };
        Ok(ResourceSchema { block })
    }
}

impl Block {
    fn empty() -> Self {
        Block {
            attributes: BTreeMap::new(),
        }
    }

    fn from_wire(type_name: &str, block: tfplugin6::schema::Block) -> Result<Self> {
        let attributes = block
            .attributes
            .into_iter()
            .map(|a| {
                let name = a.name.clone();
                Attribute::from_wire(type_name, a).map(|attr| (name, attr))
            })
            .collect::<Result<_>>()?;
        Ok(Block { attributes })
    }
}

impl Attribute {
    fn from_wire(type_name: &str, a: tfplugin6::schema::Attribute) -> Result<Self> {
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
            description: a.description,
        })
    }
}
