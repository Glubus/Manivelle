//! `#[manivelle(...)]` attributes: the schema and codec options TsSchema adds on top of
//! serde, applied to containers, fields and variants.

use syn::{Attribute, Error, LitStr, Path, Result, meta::ParseNestedMeta};

use super::{ContainerAttrs, FieldAttrs, SchemaOnly, VariantAttrs, parsed_string, string_value};
use crate::directions::Directions;

impl ContainerAttrs {
    pub(super) fn apply_manivelle(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
        if meta.path.is_ident("name") || meta.path.is_ident("rename") {
            self.schema_name = Some(string_value(meta)?);
        } else if meta.path.is_ident("codec") {
            self.json_codec = parse_json_codec(meta)?;
        } else if let Some(codecs) = restricted_codecs(&meta.path) {
            if std::mem::replace(&mut self.codecs_restricted, true) {
                return Err(
                    meta.error("use only one of `schema_only`, `encode_only` and `decode_only`")
                );
            }
            self.codecs = codecs;
        } else {
            return Err(meta.error("unsupported manivelle type attribute"));
        }
        Ok(())
    }
}

impl FieldAttrs {
    pub(super) fn apply_manivelle(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
        if meta.path.is_ident("rename") {
            self.schema_rename = Some(string_value(meta)?);
            self.schema_only.push(SchemaOnly {
                path: meta.path.clone(),
                serde_equivalent: "rename = \"...\"",
            });
        } else if meta.path.is_ident("optional") {
            self.schema_optional = true;
            self.schema_only.push(SchemaOnly {
                path: meta.path.clone(),
                serde_equivalent: "default",
            });
        } else if meta.path.is_ident("codec") {
            self.json_codec = parse_json_codec(meta)?;
        } else if meta.path.is_ident("with") {
            self.with = Some(parsed_string(meta)?);
        } else if meta.path.is_ident("type") {
            self.ts_type = Some(string_value(meta)?);
        } else {
            return Err(meta.error("unsupported manivelle field attribute"));
        }
        Ok(())
    }
}

impl VariantAttrs {
    pub(super) fn apply_manivelle(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
        if !meta.path.is_ident("rename") {
            return Err(meta.error("unsupported manivelle enum variant attribute"));
        }
        self.schema_rename = Some(string_value(meta)?);
        self.schema_only.push(SchemaOnly {
            path: meta.path.clone(),
            serde_equivalent: "rename = \"...\"",
        });
        Ok(())
    }
}

pub(super) fn parse_manivelle(
    attrs: &[Attribute],
    mut apply: impl FnMut(ParseNestedMeta<'_>) -> Result<()>,
) -> Result<()> {
    for attr in attrs
        .iter()
        .filter(|attr| attr.path().is_ident("manivelle"))
    {
        attr.parse_nested_meta(&mut apply)?;
    }
    Ok(())
}

/// Codec directions selected by `schema_only`, `encode_only` or `decode_only`.
fn restricted_codecs(path: &Path) -> Option<Directions> {
    if path.is_ident("schema_only") {
        Some(Directions::NONE)
    } else if path.is_ident("encode_only") {
        Some(Directions::ENCODE)
    } else if path.is_ident("decode_only") {
        Some(Directions::DECODE)
    } else {
        None
    }
}

fn parse_json_codec(meta: &ParseNestedMeta<'_>) -> Result<bool> {
    let codec: LitStr = meta.value()?.parse()?;
    if codec.value() == "json" {
        Ok(true)
    } else {
        Err(Error::new_spanned(
            codec,
            "unsupported manivelle codec, expected \"json\"",
        ))
    }
}
