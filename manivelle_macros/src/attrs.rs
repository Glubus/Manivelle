//! Parsing of `#[serde(...)]` and `#[manivelle(...)]` attributes.
//!
//! Serde attributes are read so the schema and the native codec follow the wire format
//! serde produces. Attributes only the serde_json reference codec can honor are kept as
//! [`JsonOnly`] entries; `check` rejects them unless the type or field opts in with
//! `#[manivelle(codec = "json")]`.

use proc_macro2::Group;
use syn::{
    Attribute, Error, Expr, ExprPath, Ident, LitStr, Path, Result, Token, Type,
    meta::ParseNestedMeta, parse::Parse, token::Paren,
};

use crate::directions::Directions;
use crate::rename::{RenameRule, RenameRules, Renamed};

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

    fn apply_manivelle(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
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

    fn check_tagging(&self, attrs: &[Attribute]) -> Result<()> {
        let message = if self.untagged && (self.tag.is_some() || self.content.is_some()) {
            "serde untagged cannot be combined with tag or content"
        } else if self.content.is_some() && self.tag.is_none() {
            "serde content requires serde tag"
        } else {
            return Ok(());
        };
        let serde_attr = attrs
            .iter()
            .find(|attr| attr.path().is_ident("serde"))
            .expect("tagging attributes come from a serde attribute");
        Err(Error::new_spanned(serde_attr, message))
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

    fn apply_manivelle(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
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

    fn apply_manivelle(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
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

/// Every serde key TsSchema reads, gathered from all `#[serde]` attributes of one item.
/// Serde itself rejects keys used in the wrong position, so one parser serves all.
#[derive(Default)]
struct SerdeAttrs {
    rename: Renamed,
    rename_all: RenameRules,
    rename_all_fields: RenameRules,
    aliases: Vec<String>,
    tag: Option<String>,
    content: Option<String>,
    untagged: Option<Path>,
    transparent: bool,
    deny_unknown_fields: bool,
    default: Option<DefaultValue>,
    skip_serializing: bool,
    skip_deserializing: bool,
    skip_serializing_if: Option<ExprPath>,
    flatten: bool,
    serialize_with: Option<ExprPath>,
    deserialize_with: Option<ExprPath>,
    into: Option<Type>,
    from: Option<Type>,
    json_only: Vec<JsonOnly>,
}

impl SerdeAttrs {
    fn parse(attrs: &[Attribute]) -> Result<Self> {
        let mut parsed = Self::default();
        for attr in attrs.iter().filter(|attr| attr.path().is_ident("serde")) {
            attr.parse_nested_meta(|meta| parsed.apply(&meta))?;
        }
        Ok(parsed)
    }

    fn apply(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
        let key = meta
            .path
            .get_ident()
            .map(Ident::to_string)
            .unwrap_or_default();
        match key.as_str() {
            "rename" => parse_renamed(meta, &mut self.rename)?,
            "rename_all" => parse_rename_rules(meta, &mut self.rename_all)?,
            "rename_all_fields" => parse_rename_rules(meta, &mut self.rename_all_fields)?,
            "alias" => self.aliases.push(string_value(meta)?),
            "tag" => self.tag = Some(string_value(meta)?),
            "content" => self.content = Some(string_value(meta)?),
            "untagged" => self.untagged = Some(meta.path.clone()),
            "transparent" => self.transparent = true,
            "deny_unknown_fields" => self.deny_unknown_fields = true,
            "default" => self.default = Some(parse_default(meta)?),
            "skip" => {
                self.skip_serializing = true;
                self.skip_deserializing = true;
            }
            "skip_serializing" => self.skip_serializing = true,
            "skip_deserializing" => self.skip_deserializing = true,
            "skip_serializing_if" => self.skip_serializing_if = Some(parsed_string(meta)?),
            "flatten" => self.flatten = true,
            "with" => self.apply_with(meta)?,
            "serialize_with" => {
                self.serialize_with = Some(parsed_string(meta)?);
                self.push_json_only(meta, Directions::ENCODE);
            }
            "deserialize_with" => {
                self.deserialize_with = Some(parsed_string(meta)?);
                self.push_json_only(meta, Directions::DECODE);
            }
            "into" => {
                self.into = Some(parsed_string(meta)?);
                self.push_json_only(meta, Directions::ENCODE);
            }
            "from" | "try_from" => {
                self.from = Some(parsed_string(meta)?);
                self.push_json_only(meta, Directions::DECODE);
            }
            "crate" | "expecting" => skip_value(meta)?,
            "other" | "borrow" => self.skip_json_only(meta, Directions::DECODE)?,
            "getter" => self.skip_json_only(meta, Directions::ENCODE)?,
            _ => self.skip_json_only(meta, Directions::BOTH)?,
        }
        Ok(())
    }

    /// `with = "module"` is `serialize_with = "module::serialize"` plus
    /// `deserialize_with = "module::deserialize"`.
    fn apply_with(&mut self, meta: &ParseNestedMeta<'_>) -> Result<()> {
        let module: ExprPath = parsed_string(meta)?;
        let function = |name: &str| {
            let mut path = module.clone();
            path.path
                .segments
                .push(Ident::new(name, proc_macro2::Span::call_site()).into());
            path
        };
        self.serialize_with = Some(function("serialize"));
        self.deserialize_with = Some(function("deserialize"));
        self.push_json_only(meta, Directions::BOTH);
        Ok(())
    }

    fn skip_json_only(&mut self, meta: &ParseNestedMeta<'_>, directions: Directions) -> Result<()> {
        self.push_json_only(meta, directions);
        skip_value(meta)
    }

    fn push_json_only(&mut self, meta: &ParseNestedMeta<'_>, directions: Directions) {
        self.json_only.push(JsonOnly {
            path: meta.path.clone(),
            directions,
        });
    }
}

fn parse_manivelle(
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

/// `rename = "name"` or `rename(serialize = "a", deserialize = "b")`.
fn parse_renamed(meta: &ParseNestedMeta<'_>, renamed: &mut Renamed) -> Result<()> {
    if meta.input.peek(Token![=]) {
        let name = string_value(meta)?;
        renamed.serialize = Some(name.clone());
        renamed.deserialize = Some(name);
        return Ok(());
    }
    meta.parse_nested_meta(|direction| {
        let name = Some(string_value(&direction)?);
        if direction.path.is_ident("serialize") {
            renamed.serialize = name;
        } else if direction.path.is_ident("deserialize") {
            renamed.deserialize = name;
        } else {
            return Err(direction.error("expected `serialize` or `deserialize`"));
        }
        Ok(())
    })
}

/// `rename_all = "rule"` or `rename_all(serialize = "a", deserialize = "b")`.
fn parse_rename_rules(meta: &ParseNestedMeta<'_>, rules: &mut RenameRules) -> Result<()> {
    if meta.input.peek(Token![=]) {
        let rule = RenameRule::parse(&meta.value()?.parse()?)?;
        rules.serialize = Some(rule);
        rules.deserialize = Some(rule);
        return Ok(());
    }
    meta.parse_nested_meta(|direction| {
        let rule = Some(RenameRule::parse(&direction.value()?.parse()?)?);
        if direction.path.is_ident("serialize") {
            rules.serialize = rule;
        } else if direction.path.is_ident("deserialize") {
            rules.deserialize = rule;
        } else {
            return Err(direction.error("expected `serialize` or `deserialize`"));
        }
        Ok(())
    })
}

fn parse_default(meta: &ParseNestedMeta<'_>) -> Result<DefaultValue> {
    if meta.input.peek(Token![=]) {
        Ok(DefaultValue::Path(parsed_string(meta)?))
    } else {
        Ok(DefaultValue::Trait)
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
