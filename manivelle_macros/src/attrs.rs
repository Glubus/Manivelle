//! Parsing of `#[serde(...)]` and `#[manivelle(...)]` attributes.
//!
//! Serde attributes are read so the schema and the native codec follow the wire format
//! serde produces. Attributes only the serde_json reference codec can honor are kept as
//! [`JsonOnly`] entries; `check` rejects them unless the type or field opts in with
//! `#[manivelle(codec = "json")]`.
mod manivelle_attrs;
mod serde_attrs;

use proc_macro2::{Group, Span};
use syn::{
    Attribute, Error, Expr, ExprPath, LitStr, Path, Result, Token, Type, meta::ParseNestedMeta,
    parse::Parse, token::Paren,
};

use self::manivelle_attrs::parse_manivelle;
use self::serde_attrs::SerdeAttrs;
use crate::directions::Directions;
use crate::rename::{RenameRules, Renamed};

/// `#[serde(default)]` or `#[serde(default = "path")]`.
pub(crate) enum DefaultValue {
    Trait,
    Path(ExprPath),
}

/// A serde attribute only the serde_json reference codec can honor, with the codec
/// directions it changes.
pub(crate) struct JsonOnly {
    pub(crate) path: Path,
    pub(crate) directions: Directions,
}

/// A `#[manivelle]` attribute that changes only the schema, with the serde attribute that
/// changes schema and codec together.
pub(crate) struct SchemaOnly {
    pub(crate) path: Path,
    pub(crate) serde_equivalent: &'static str,
}

/// Type-level attributes.
#[derive(Default)]
pub(crate) struct ContainerAttrs {
    pub(crate) schema_name: Option<String>,
    pub(crate) json_codec: bool,
    /// Codecs to emit: both by default, restricted by `#[manivelle(encode_only)]`,
    /// `#[manivelle(decode_only)]` or `#[manivelle(schema_only)]`.
    pub(crate) codecs: Directions,
    codecs_restricted: bool,
    pub(crate) rename_all: RenameRules,
    pub(crate) rename_all_fields: RenameRules,
    pub(crate) tag: Option<String>,
    pub(crate) content: Option<String>,
    pub(crate) untagged: bool,
    pub(crate) transparent: bool,
    pub(crate) deny_unknown_fields: bool,
    pub(crate) default: Option<DefaultValue>,
    pub(crate) into: Option<Type>,
    pub(crate) from: Option<Type>,
    pub(crate) json_only: Vec<JsonOnly>,
}

impl ContainerAttrs {
    pub(crate) fn from_attrs(attrs: &[Attribute]) -> Result<Self> {
        let serde = SerdeAttrs::parse(attrs)?;
        let mut container = Self {
            codecs: Directions::BOTH,
            rename_all: serde.rename_all,
            rename_all_fields: serde.rename_all_fields,
            tag: serde.tag,
            content: serde.content,
            untagged: serde.untagged.is_some(),
            transparent: serde.transparent,
            deny_unknown_fields: serde.deny_unknown_fields,
            default: serde.default,
            into: serde.into,
            from: serde.from,
            json_only: serde.json_only,
            ..Self::default()
        };
        parse_manivelle(attrs, |meta| container.apply_manivelle(&meta))?;
        container.check_tagging(attrs)?;
        Ok(container)
    }

    fn check_tagging(&self, attrs: &[Attribute]) -> Result<()> {
        let message = if self.untagged && (self.tag.is_some() || self.content.is_some()) {
            "serde untagged cannot be combined with tag or content"
        } else if self.content.is_some() && self.tag.is_none() {
            "serde content requires serde tag"
        } else {
            return Ok(());
        };
        Err(
            match attrs.iter().find(|attr| attr.path().is_ident("serde")) {
                Some(serde_attr) => Error::new_spanned(serde_attr, message),
                None => Error::new(Span::call_site(), message),
            },
        )
    }
}

/// Field-level attributes.
#[derive(Default)]
pub(crate) struct FieldAttrs {
    pub(crate) rename: Renamed,
    pub(crate) aliases: Vec<String>,
    pub(crate) default: Option<DefaultValue>,
    pub(crate) skip_serializing: bool,
    pub(crate) skip_deserializing: bool,
    pub(crate) skip_serializing_if: Option<ExprPath>,
    pub(crate) flatten: bool,
    pub(crate) serialize_with: Option<ExprPath>,
    pub(crate) deserialize_with: Option<ExprPath>,
    pub(crate) json_only: Vec<JsonOnly>,
    /// `#[manivelle(codec = "json")]`: this field crosses through serde_json.
    pub(crate) json_codec: bool,
    /// `#[manivelle(with = "path")]`: `path::encode_js` / `path::decode_js` codec.
    pub(crate) with: Option<ExprPath>,
    /// `#[manivelle(type = "...")]`: TypeScript type text used instead of the field's schema.
    pub(crate) ts_type: Option<String>,
    pub(crate) schema_rename: Option<String>,
    pub(crate) schema_optional: bool,
    pub(crate) schema_only: Vec<SchemaOnly>,
}

impl FieldAttrs {
    pub(crate) fn from_field(field: &syn::Field) -> Result<Self> {
        let serde = SerdeAttrs::parse(&field.attrs)?;
        let mut attrs = Self {
            rename: serde.rename,
            aliases: serde.aliases,
            default: serde.default,
            skip_serializing: serde.skip_serializing,
            skip_deserializing: serde.skip_deserializing,
            skip_serializing_if: serde.skip_serializing_if,
            flatten: serde.flatten,
            serialize_with: serde.serialize_with,
            deserialize_with: serde.deserialize_with,
            json_only: serde.json_only,
            ..Self::default()
        };
        parse_manivelle(&field.attrs, |meta| attrs.apply_manivelle(&meta))?;
        Ok(attrs)
    }
}

/// Enum variant attributes.
#[derive(Default)]
pub(crate) struct VariantAttrs {
    pub(crate) rename: Renamed,
    pub(crate) aliases: Vec<String>,
    pub(crate) rename_all: RenameRules,
    pub(crate) skip_serializing: bool,
    pub(crate) skip_deserializing: bool,
    pub(crate) json_only: Vec<JsonOnly>,
    pub(crate) schema_rename: Option<String>,
    pub(crate) schema_only: Vec<SchemaOnly>,
}

impl VariantAttrs {
    pub(crate) fn from_variant(variant: &syn::Variant) -> Result<Self> {
        let serde = SerdeAttrs::parse(&variant.attrs)?;
        let mut json_only = serde.json_only;
        if let Some(path) = serde.untagged {
            json_only.push(JsonOnly {
                path,
                directions: Directions::BOTH,
            });
        }
        let mut attrs = Self {
            rename: serde.rename,
            aliases: serde.aliases,
            rename_all: serde.rename_all,
            skip_serializing: serde.skip_serializing,
            skip_deserializing: serde.skip_deserializing,
            json_only,
            ..Self::default()
        };
        parse_manivelle(&variant.attrs, |meta| attrs.apply_manivelle(&meta))?;
        Ok(attrs)
    }
}

fn string_value(meta: &ParseNestedMeta<'_>) -> Result<String> {
    Ok(meta.value()?.parse::<LitStr>()?.value())
}

/// Parses the contents of a string literal value, like serde's `"path"` arguments.
fn parsed_string<T: Parse>(meta: &ParseNestedMeta<'_>) -> Result<T> {
    meta.value()?.parse::<LitStr>()?.parse()
}

/// Consumes `= value` or `(...)` of an attribute whose content is not needed.
fn skip_value(meta: &ParseNestedMeta<'_>) -> Result<()> {
    if meta.input.peek(Token![=]) {
        meta.value()?.parse::<Expr>()?;
    } else if meta.input.peek(Paren) {
        meta.input.parse::<Group>()?;
    }
    Ok(())
}
