//! Serde-compatible naming: `rename_all` rules and per-direction field/variant names.

use syn::{Error, Ident, LitStr, Result};

/// A serde `rename_all` case rule. Conversions match `serde_derive` exactly, so the
/// schema and the codec use the same keys serde does.
#[derive(Clone, Copy)]
pub(crate) enum RenameRule {
    Lower,
    Upper,
    Pascal,
    Camel,
    Snake,
    ScreamingSnake,
    Kebab,
    ScreamingKebab,
}

impl RenameRule {
    pub(crate) fn parse(lit: &LitStr) -> Result<Self> {
        match lit.value().as_str() {
            "lowercase" => Ok(Self::Lower),
            "UPPERCASE" => Ok(Self::Upper),
            "PascalCase" => Ok(Self::Pascal),
            "camelCase" => Ok(Self::Camel),
            "snake_case" => Ok(Self::Snake),
            "SCREAMING_SNAKE_CASE" => Ok(Self::ScreamingSnake),
            "kebab-case" => Ok(Self::Kebab),
            "SCREAMING-KEBAB-CASE" => Ok(Self::ScreamingKebab),
            _ => Err(Error::new_spanned(lit, "unsupported serde rename_all rule")),
        }
    }

    /// Renames a `snake_case` field identifier.
    fn apply_to_field(self, field: &str) -> String {
        match self {
            Self::Lower | Self::Snake => field.to_owned(),
            Self::Upper | Self::ScreamingSnake => field.to_ascii_uppercase(),
            Self::Pascal => pascal_from_snake(field),
            Self::Camel => lower_first(&pascal_from_snake(field)),
            Self::Kebab => field.replace('_', "-"),
            Self::ScreamingKebab => field.to_ascii_uppercase().replace('_', "-"),
        }
    }

    /// Renames a `PascalCase` variant identifier.
    fn apply_to_variant(self, variant: &str) -> String {
        match self {
            Self::Pascal => variant.to_owned(),
            Self::Lower => variant.to_ascii_lowercase(),
            Self::Upper => variant.to_ascii_uppercase(),
            Self::Camel => lower_first(variant),
            Self::Snake => snake_from_pascal(variant),
            Self::ScreamingSnake => snake_from_pascal(variant).to_ascii_uppercase(),
            Self::Kebab => snake_from_pascal(variant).replace('_', "-"),
            Self::ScreamingKebab => snake_from_pascal(variant)
                .to_ascii_uppercase()
                .replace('_', "-"),
        }
    }
}

fn pascal_from_snake(field: &str) -> String {
    let mut pascal = String::new();
    let mut capitalize = true;
    for character in field.chars() {
        if character == '_' {
            capitalize = true;
        } else if capitalize {
            pascal.push(character.to_ascii_uppercase());
            capitalize = false;
        } else {
            pascal.push(character);
        }
    }
    pascal
}

fn snake_from_pascal(variant: &str) -> String {
    let mut snake = String::new();
    for (index, character) in variant.char_indices() {
        if index > 0 && character.is_uppercase() {
            snake.push('_');
        }
        snake.push(character.to_ascii_lowercase());
    }
    snake
}

fn lower_first(value: &str) -> String {
    let mut characters = value.chars();
    match characters.next() {
        Some(first) => first.to_ascii_lowercase().to_string() + characters.as_str(),
        None => String::new(),
    }
}

/// `rename_all` rules per direction: `rename_all = "..."` sets both,
/// `rename_all(serialize = "...", deserialize = "...")` sets them separately.
#[derive(Clone, Copy, Default)]
pub(crate) struct RenameRules {
    pub(crate) serialize: Option<RenameRule>,
    pub(crate) deserialize: Option<RenameRule>,
}

impl RenameRules {
    /// Uses the rules set on `self` and `fallback` for the others, like a variant's
    /// `rename_all` over its container's `rename_all_fields`.
    pub(crate) fn or(self, fallback: Self) -> Self {
        Self {
            serialize: self.serialize.or(fallback.serialize),
            deserialize: self.deserialize.or(fallback.deserialize),
        }
    }
}

/// An explicit serde `rename` per direction.
#[derive(Clone, Default)]
pub(crate) struct Renamed {
    pub(crate) serialize: Option<String>,
    pub(crate) deserialize: Option<String>,
}

/// Serde name of a field or variant in each direction, plus deserialize-only aliases.
pub(crate) struct Name {
    pub(crate) serialize: String,
    pub(crate) deserialize: String,
    pub(crate) aliases: Vec<String>,
}

impl Name {
    pub(crate) fn field(
        ident: &Ident,
        renamed: &Renamed,
        rules: RenameRules,
        aliases: &[String],
    ) -> Self {
        Self::resolve(ident, renamed, rules, aliases, RenameRule::apply_to_field)
    }

    pub(crate) fn variant(
        ident: &Ident,
        renamed: &Renamed,
        rules: RenameRules,
        aliases: &[String],
    ) -> Self {
        Self::resolve(ident, renamed, rules, aliases, RenameRule::apply_to_variant)
    }

    /// Name of a tuple field, which serde never renames.
    pub(crate) fn index(index: usize) -> Self {
        Self {
            serialize: index.to_string(),
            deserialize: index.to_string(),
            aliases: Vec::new(),
        }
    }

    /// Every key accepted on decode: the deserialize name first, then the aliases.
    pub(crate) fn accepted(&self) -> impl Iterator<Item = &str> {
        std::iter::once(self.deserialize.as_str()).chain(self.aliases.iter().map(String::as_str))
    }

    fn resolve(
        ident: &Ident,
        renamed: &Renamed,
        rules: RenameRules,
        aliases: &[String],
        apply: fn(RenameRule, &str) -> String,
    ) -> Self {
        let original = ident.to_string();
        let original = original.strip_prefix("r#").unwrap_or(&original);
        let directional = |explicit: &Option<String>, rule: Option<RenameRule>| {
            explicit.clone().unwrap_or_else(|| match rule {
                Some(rule) => apply(rule, original),
                None => original.to_owned(),
            })
        };
        Self {
            serialize: directional(&renamed.serialize, rules.serialize),
            deserialize: directional(&renamed.deserialize, rules.deserialize),
            aliases: aliases.to_vec(),
        }
    }
}
