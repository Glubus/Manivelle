//! `#[serde(...)]` attributes: every key TsSchema reads, gathered from all `#[serde]`
//! attributes of one item.

use syn::{Attribute, ExprPath, Ident, Path, Result, Token, Type, meta::ParseNestedMeta};

use super::{DefaultValue, JsonOnly, parsed_string, skip_value, string_value};
use crate::directions::Directions;
use crate::rename::{RenameRule, RenameRules, Renamed};

/// Every serde key TsSchema reads, gathered from all `#[serde]` attributes of one item.
/// Serde itself rejects keys used in the wrong position, so one parser serves all.
#[derive(Default)]
pub(super) struct SerdeAttrs {
    pub(super) rename: Renamed,
    pub(super) rename_all: RenameRules,
    pub(super) rename_all_fields: RenameRules,
    pub(super) aliases: Vec<String>,
    pub(super) tag: Option<String>,
    pub(super) content: Option<String>,
    pub(super) untagged: Option<Path>,
    pub(super) transparent: bool,
    pub(super) deny_unknown_fields: bool,
    pub(super) default: Option<DefaultValue>,
    pub(super) skip_serializing: bool,
    pub(super) skip_deserializing: bool,
    pub(super) skip_serializing_if: Option<ExprPath>,
    pub(super) flatten: bool,
    pub(super) serialize_with: Option<ExprPath>,
    pub(super) deserialize_with: Option<ExprPath>,
    pub(super) into: Option<Type>,
    pub(super) from: Option<Type>,
    pub(super) json_only: Vec<JsonOnly>,
}

impl SerdeAttrs {
    pub(super) fn parse(attrs: &[Attribute]) -> Result<Self> {
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
