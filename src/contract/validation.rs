//! Runtime validation for host contract schemas.
//!
//! A value is read where it lives, through [`Inspect`]: a JSON value or a QuickJS value
//! is walked as far as the schema describes it, and nothing is copied out of it first.

use std::borrow::Cow;
use std::collections::HashMap;
use std::convert::Infallible;
use std::fmt;

use serde_json::Value;

use super::codec::is_canonical_integer;
use super::{Schema, TsEnumVariant, TsField, TsLiteral, TsRecordKey, TsType};

/// The kinds of value validation tells apart.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Null,
    Boolean,
    Number,
    String,
    /// An array, or a byte buffer, which reads as an array of numbers.
    Array,
    Object,
    /// A value no schema describes, by its name: a symbol, a BigInt.
    Other(&'static str),
}

impl Kind {
    /// The name `expected` messages give this kind.
    fn label(self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Boolean => "boolean",
            Self::Number => "number",
            Self::String => "string",
            Self::Array => "array",
            Self::Object => "object",
            Self::Other(name) => name,
        }
    }
}

/// What validation reads of a value, in place. A JSON value never fails to be read; a
/// QuickJS value can, when reading it runs script code that throws.
pub(crate) trait Inspect: Sized {
    /// Why a value could not be read.
    type Error;

    fn kind(&self) -> Kind;

    fn boolean(&self) -> Option<bool>;

    fn number(&self) -> Option<f64>;

    /// The text of a string, `None` for any other kind. Asked only where a schema names
    /// the text it expects (a literal, an enum variant), so a plain `string` copies nothing.
    fn text(&self) -> Result<Option<Cow<'_, str>>, Self::Error>;

    /// Whether the value is a byte buffer: every item of it is a number.
    fn is_bytes(&self) -> bool;

    /// How many items an array or a byte buffer holds, `None` for any other kind.
    fn len(&self) -> Result<Option<usize>, Self::Error>;

    /// The item at `index` of an array or a byte buffer.
    fn item(&self, index: usize) -> Result<Self, Self::Error>;

    /// The value of the object's own property `name`, `None` when it has none.
    fn field(&self, name: &str) -> Result<Option<Self>, Self::Error>;

    /// Hands each own property of the object to `visit`, in insertion order, until one
    /// fails.
    fn entries(&self, visit: &mut Visit<'_, Self>) -> Result<(), Invalid<Self::Error>>;
}

/// What [`Inspect::entries`] hands each property to: its key and its value.
pub(crate) type Visit<'a, V> =
    dyn FnMut(&str, V) -> Result<(), Invalid<<V as Inspect>::Error>> + 'a;

/// Why a validation did not pass.
#[derive(Debug)]
pub(crate) enum Invalid<E> {
    /// The value does not match the schema: the path and the reason.
    Mismatch(String),
    /// The value could not be read.
    Read(E),
}

impl<E> From<E> for Invalid<E> {
    fn from(error: E) -> Self {
        Self::Read(error)
    }
}

impl Invalid<Infallible> {
    /// The mismatch: a value that is always readable fails for no other reason.
    pub(crate) fn into_mismatch(self) -> String {
        match self {
            Self::Mismatch(details) => details,
            Self::Read(never) => match never {},
        }
    }
}

static JSON_NULL: Value = Value::Null;

impl Inspect for &Value {
    type Error = Infallible;

    fn kind(&self) -> Kind {
        match self {
            Value::Null => Kind::Null,
            Value::Bool(_) => Kind::Boolean,
            Value::Number(_) => Kind::Number,
            Value::String(_) => Kind::String,
            Value::Array(_) => Kind::Array,
            Value::Object(_) => Kind::Object,
        }
    }

    fn boolean(&self) -> Option<bool> {
        self.as_bool()
    }

    fn number(&self) -> Option<f64> {
        // `as_f64` fails only for arbitrary-precision numbers beyond the `f64` range.
        self.as_number()
            .map(|number| number.as_f64().unwrap_or(f64::NAN))
    }

    fn text(&self) -> Result<Option<Cow<'_, str>>, Infallible> {
        Ok(self.as_str().map(Cow::Borrowed))
    }

    fn is_bytes(&self) -> bool {
        false
    }

    fn len(&self) -> Result<Option<usize>, Infallible> {
        Ok(self.as_array().map(Vec::len))
    }

    fn item(&self, index: usize) -> Result<Self, Infallible> {
        Ok((*self).get(index).unwrap_or(&JSON_NULL))
    }

    fn field(&self, name: &str) -> Result<Option<Self>, Infallible> {
        Ok((*self).as_object().and_then(|entries| entries.get(name)))
    }

    fn entries(&self, visit: &mut Visit<'_, Self>) -> Result<(), Invalid<Infallible>> {
        for (key, value) in (*self).as_object().into_iter().flatten() {
            visit(key, value)?;
        }
        Ok(())
    }
}

pub(crate) fn validate_schema_with_options<V: Inspect>(
    schema: &Schema,
    value: &V,
    options: SchemaValidationOptions,
) -> Result<(), Invalid<V::Error>> {
    let mut context = ValidationContext::new(schema).map_err(Invalid::Mismatch)?;
    validate_type(&schema.ts_type, value, &Path::Root, options, &mut context)
}

#[derive(Debug, Clone, Copy, Default)]
pub(crate) struct SchemaValidationOptions {
    pub(crate) reject_unknown_fields: bool,
}

/// The location of a value, formatted only when a message needs it.
#[derive(Clone, Copy)]
enum Path<'a> {
    Root,
    Field(&'a Path<'a>, &'a str),
    Index(&'a Path<'a>, usize),
}

impl fmt::Display for Path<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Root => f.write_str("$"),
            Self::Field(parent, name) => write!(f, "{parent}.{name}"),
            Self::Index(parent, index) => write!(f, "{parent}[{index}]"),
        }
    }
}

/// A named schema dependency: its slot in the context, and its schema.
#[derive(Clone, Copy)]
struct Dependency<'a> {
    id: usize,
    schema: &'a Schema,
}

struct ValidationContext<'a> {
    /// The first dependency with each name, in depth-first order.
    dependencies: HashMap<&'a str, Dependency<'a>>,
    /// The dependencies being validated, so that a recursive reference stops. It also
    /// bounds the walk: a value is followed only as deep as the schema goes, however
    /// deep or cyclic the value itself is.
    resolving: Vec<usize>,
}

impl<'a> ValidationContext<'a> {
    fn new(schema: &'a Schema) -> Result<Self, String> {
        let mut dependencies = HashMap::new();
        collect_dependencies(&schema.dependencies, &mut dependencies)?;
        Ok(Self {
            dependencies,
            resolving: Vec::new(),
        })
    }
}

/// Records each named dependency once. Two with the same name must have the same shape:
/// a type reference is resolved by name, so the second would be validated and declared
/// as the first.
fn collect_dependencies<'a>(
    schemas: &'a [Schema],
    dependencies: &mut HashMap<&'a str, Dependency<'a>>,
) -> Result<(), String> {
    for schema in schemas {
        if let Some(name) = schema.declared_name() {
            match dependencies.get(name) {
                Some(known) if known.schema != schema => {
                    return Err(format!(
                        "two schemas are named `{name}` but have different shapes"
                    ));
                }
                Some(_) => {}
                None => {
                    let id = dependencies.len();
                    dependencies.insert(name, Dependency { id, schema });
                }
            }
        }
        collect_dependencies(&schema.dependencies, dependencies)?;
    }
    Ok(())
}

fn validate_type<V: Inspect>(
    ty: &TsType,
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    match ty {
        TsType::Unknown | TsType::Json => Ok(()),
        TsType::TypeRef(name) => validate_type_ref(name, value, path, options, context),
        TsType::Void | TsType::Null => expect_kind(value, Kind::Null, path),
        TsType::Boolean => expect_kind(value, Kind::Boolean, path),
        TsType::Number => expect_kind(value, Kind::Number, path),
        TsType::String => expect_kind(value, Kind::String, path),
        // Every item of a byte buffer is a number already: nothing to look at.
        TsType::Uint8Array if value.is_bytes() => Ok(()),
        TsType::Uint8Array => validate_array(&TsType::Number, value, path, options, context),
        TsType::Literal(literal) => validate_literal(literal, value, path),
        TsType::Object(fields) => {
            validate_object(ObjectShape::new(fields), value, path, options, context)
        }
        TsType::OpenObject { fields, rest } => validate_object(
            ObjectShape::new(fields).with_rest(Some(rest.as_ref())),
            value,
            path,
            options,
            context,
        ),
        TsType::Array(item) => validate_array(item, value, path, options, context),
        TsType::Tuple(items) => validate_tuple(items, value, path, options, context),
        TsType::Enum { tag, variants } => {
            validate_enum(tag.as_deref(), variants, value, path, options, context)
        }
        TsType::Record { key, value: item } => {
            validate_record(*key, item, value, path, options, context)
        }
        TsType::Union(types) => validate_union(types, value, path, options, context),
        TsType::Optional(inner) | TsType::Nullable(inner) => {
            validate_nullish(inner, value, path, options, context)
        }
    }
}

fn validate_type_ref<V: Inspect>(
    name: &str,
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    let Some(&Dependency { id, schema }) = context.dependencies.get(name) else {
        return Ok(());
    };
    if context.resolving.contains(&id) {
        return Ok(());
    }

    context.resolving.push(id);
    let result = validate_type(&schema.ts_type, value, path, options, context);
    context.resolving.pop();
    result
}

fn expect_kind<V: Inspect>(
    value: &V,
    kind: Kind,
    path: &Path<'_>,
) -> Result<(), Invalid<V::Error>> {
    if value.kind() == kind {
        Ok(())
    } else {
        Err(expected(path, kind.label(), value))
    }
}

fn validate_literal<V: Inspect>(
    literal: &TsLiteral,
    value: &V,
    path: &Path<'_>,
) -> Result<(), Invalid<V::Error>> {
    let valid = match literal {
        TsLiteral::String(expected) => value.text()?.is_some_and(|text| text == expected.as_str()),
        TsLiteral::Number(expected) => value
            .number()
            .is_some_and(|number| literal_number_matches(expected, number)),
        TsLiteral::Boolean(expected) => value.boolean() == Some(*expected),
    };
    if valid {
        Ok(())
    } else {
        Err(Invalid::Mismatch(format!(
            "{path}: expected literal {}",
            literal_label(literal)
        )))
    }
}

fn literal_number_matches(expected: &str, number: f64) -> bool {
    expected
        .parse::<f64>()
        .is_ok_and(|expected| number == expected)
}

fn literal_label(literal: &TsLiteral) -> String {
    match literal {
        TsLiteral::String(value) => format!("{value:?}"),
        TsLiteral::Number(value) => value.clone(),
        TsLiteral::Boolean(value) => value.to_string(),
    }
}

/// Keys an object schema describes: declared fields, keys allowed next to them (an enum
/// tag), and the type of every other key when the object is open.
#[derive(Clone, Copy)]
struct ObjectShape<'a> {
    fields: &'a [TsField],
    allowed_extra_fields: &'a [&'a str],
    rest: Option<&'a TsType>,
}

impl<'a> ObjectShape<'a> {
    fn new(fields: &'a [TsField]) -> Self {
        Self {
            fields,
            allowed_extra_fields: &[],
            rest: None,
        }
    }

    fn with_rest(self, rest: Option<&'a TsType>) -> Self {
        Self { rest, ..self }
    }

    fn with_allowed_extra_fields(self, allowed_extra_fields: &'a [&'a str]) -> Self {
        Self {
            allowed_extra_fields,
            ..self
        }
    }

    /// Whether `key` is neither a declared field nor an allowed extra key.
    fn is_undeclared(&self, key: &str) -> bool {
        !self.fields.iter().any(|declared| declared.name == key)
            && !self.allowed_extra_fields.contains(&key)
    }
}

fn validate_object<V: Inspect>(
    shape: ObjectShape<'_>,
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    if value.kind() != Kind::Object {
        return Err(expected(path, "object", value));
    }

    if shape.rest.is_none() && options.reject_unknown_fields {
        reject_unknown_object_fields(shape, value, path)?;
    }
    validate_declared_fields(shape.fields, value, path, options, context)?;
    match shape.rest {
        Some(rest) => validate_rest_fields(shape, rest, value, path, options, context),
        None => Ok(()),
    }
}

fn validate_declared_fields<V: Inspect>(
    fields: &[TsField],
    object: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    for field in fields {
        let field_path = Path::Field(path, field.name.as_str());
        match object.field(&field.name)? {
            Some(value) => validate_type(&field.ty, &value, &field_path, options, context)?,
            None if field.optional => {}
            None => {
                return Err(Invalid::Mismatch(format!(
                    "{field_path}: missing required field"
                )));
            }
        }
    }
    Ok(())
}

/// Validates every undeclared key of an open object against its `rest` type.
fn validate_rest_fields<V: Inspect>(
    shape: ObjectShape<'_>,
    rest: &TsType,
    object: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    object.entries(&mut |key, item| {
        if shape.is_undeclared(key) {
            validate_type(rest, &item, &Path::Field(path, key), options, context)?;
        }
        Ok(())
    })
}

fn reject_unknown_object_fields<V: Inspect>(
    shape: ObjectShape<'_>,
    object: &V,
    path: &Path<'_>,
) -> Result<(), Invalid<V::Error>> {
    object.entries(&mut |key, _| {
        if shape.is_undeclared(key) {
            return Err(Invalid::Mismatch(format!(
                "{}: unknown field",
                Path::Field(path, key)
            )));
        }
        Ok(())
    })
}

fn validate_array<V: Inspect>(
    item: &TsType,
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    let Some(len) = value.len()? else {
        return Err(expected(path, "array", value));
    };

    for index in 0..len {
        let element = value.item(index)?;
        validate_type(item, &element, &Path::Index(path, index), options, context)?;
    }
    Ok(())
}

fn validate_tuple<V: Inspect>(
    items: &[TsType],
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    let Some(len) = value.len()? else {
        return Err(expected(path, "tuple array", value));
    };
    if len != items.len() {
        return Err(Invalid::Mismatch(format!(
            "{path}: expected tuple length {}, got {len}",
            items.len()
        )));
    }

    for (index, ty) in items.iter().enumerate() {
        let element = value.item(index)?;
        validate_type(ty, &element, &Path::Index(path, index), options, context)?;
    }
    Ok(())
}

fn validate_enum<V: Inspect>(
    tag: Option<&str>,
    variants: &[TsEnumVariant],
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    match tag {
        Some(tag) => validate_tagged_enum(tag, variants, value, path, options, context),
        None => validate_unit_enum(variants, value, path),
    }
}

fn validate_unit_enum<V: Inspect>(
    variants: &[TsEnumVariant],
    value: &V,
    path: &Path<'_>,
) -> Result<(), Invalid<V::Error>> {
    let Some(name) = value.text()? else {
        return Err(expected(path, "enum string literal", value));
    };
    if variants.iter().any(|variant| variant.name == *name) {
        Ok(())
    } else {
        Err(Invalid::Mismatch(format!(
            "{path}: unknown enum variant {name:?}"
        )))
    }
}

fn validate_tagged_enum<V: Inspect>(
    tag: &str,
    variants: &[TsEnumVariant],
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    if value.kind() != Kind::Object {
        return Err(expected(path, "tagged enum object", value));
    }
    let tag_path = Path::Field(path, tag);
    let tag_field = value.field(tag)?;
    let tag_text = match &tag_field {
        Some(field) => field.text()?,
        None => None,
    };
    let Some(tag_value) = tag_text else {
        return Err(Invalid::Mismatch(format!(
            "{tag_path}: missing enum tag string"
        )));
    };
    let Some(variant) = variants.iter().find(|variant| variant.name == *tag_value) else {
        return Err(Invalid::Mismatch(format!(
            "{tag_path}: unknown enum variant {tag_value:?}"
        )));
    };

    let allowed_extra_fields = [tag];
    let shape = ObjectShape::new(&variant.fields)
        .with_rest(variant.rest.as_deref())
        .with_allowed_extra_fields(&allowed_extra_fields);
    validate_object(shape, value, path, options, context)
}

fn validate_record<V: Inspect>(
    key: TsRecordKey,
    item: &TsType,
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    if value.kind() != Kind::Object {
        return Err(expected(path, "record object", value));
    }

    value.entries(&mut |field, field_value| {
        validate_record_key(key, field, path).map_err(Invalid::Mismatch)?;
        validate_type(
            item,
            &field_value,
            &Path::Field(path, field),
            options,
            context,
        )
    })
}

fn validate_record_key(key: TsRecordKey, field: &str, path: &Path<'_>) -> Result<(), String> {
    match key {
        TsRecordKey::String => Ok(()),
        TsRecordKey::Number => is_canonical_integer(field)
            .then_some(())
            .ok_or_else(|| format!("{path}: expected numeric record key, got {field:?}")),
    }
}

/// Accepts a value that some member accepts. Otherwise the message keeps every member's
/// own error, so the mismatch is visible.
fn validate_union<V: Inspect>(
    types: &[TsType],
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    let mut member_errors = Vec::with_capacity(types.len());
    for ty in types {
        match validate_type(ty, value, path, options, context) {
            Ok(()) => return Ok(()),
            Err(Invalid::Mismatch(error)) => member_errors.push(error),
            Err(unreadable) => return Err(unreadable),
        }
    }

    Err(Invalid::Mismatch(format!(
        "{path}: value did not match any union member ({})",
        member_errors.join("; ")
    )))
}

/// Accepts `null` for an optional or nullable type, and validates anything else as `inner`.
fn validate_nullish<V: Inspect>(
    inner: &TsType,
    value: &V,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), Invalid<V::Error>> {
    if value.kind() == Kind::Null {
        return Ok(());
    }
    validate_type(inner, value, path, options, context)
}

fn expected<V: Inspect>(path: &Path<'_>, expected: &str, value: &V) -> Invalid<V::Error> {
    Invalid::Mismatch(format!(
        "{path}: expected {expected}, got {}",
        value.kind().label()
    ))
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    /// Validates a JSON value, which is always readable: the error is the mismatch.
    fn validate(
        schema: &Schema,
        value: &Value,
        options: SchemaValidationOptions,
    ) -> Result<(), String> {
        validate_schema_with_options(schema, &value, options).map_err(Invalid::into_mismatch)
    }

    #[test]
    fn validates_nested_object_shape() {
        let schema = Schema::typed(
            "Input",
            TsType::Object(vec![
                TsField::required("id", TsType::Number),
                TsField::optional("tag", TsType::String),
            ]),
        );
        let options = SchemaValidationOptions::default();

        assert!(validate(&schema, &json!({ "id": 1 }), options).is_ok());
        assert!(validate(&schema, &json!({ "tag": "x" }), options).is_err());
        assert!(validate(&schema, &json!({ "id": "1" }), options).is_err());
    }

    #[test]
    fn strict_validation_rejects_unknown_object_fields() {
        let schema = Schema::typed(
            "Input",
            TsType::Object(vec![
                TsField::required("id", TsType::Number),
                TsField::required(
                    "profile",
                    TsType::Object(vec![TsField::required("name", TsType::String)]),
                ),
            ]),
        );
        let options = SchemaValidationOptions {
            reject_unknown_fields: true,
        };

        let error = validate(
            &schema,
            &json!({
                "id": 1,
                "profile": {
                    "name": "Ada",
                    "role": "admin"
                }
            }),
            options,
        )
        .expect_err("unknown nested field should fail");

        assert_eq!(error, "$.profile.role: unknown field");
    }

    #[test]
    fn strict_validation_allows_tagged_enum_discriminant() {
        let schema = Schema::typed(
            "Event",
            TsType::Enum {
                tag: Some(String::from("type")),
                variants: vec![TsEnumVariant::payload(
                    "damage",
                    vec![TsField::required("amount", TsType::Number)],
                )],
            },
        );
        let options = SchemaValidationOptions {
            reject_unknown_fields: true,
        };

        assert!(validate(&schema, &json!({ "type": "damage", "amount": 12 }), options).is_ok());
        assert!(
            validate(
                &schema,
                &json!({ "type": "damage", "amount": 12, "debug": true }),
                options,
            )
            .is_err()
        );
    }

    #[test]
    fn validates_union_and_nullable_shapes() {
        let schema = Schema::typed(
            "Input",
            TsType::Union(vec![
                TsType::String,
                TsType::Nullable(Box::new(TsType::Number)),
            ]),
        );
        let options = SchemaValidationOptions::default();

        assert!(validate(&schema, &json!("ok"), options).is_ok());
        assert!(validate(&schema, &json!(42), options).is_ok());
        assert!(validate(&schema, &Value::Null, options).is_ok());
        assert!(validate(&schema, &json!(true), options).is_err());
    }

    #[test]
    fn union_error_reports_each_member_mismatch() {
        let schema = Schema::typed(
            "Input",
            TsType::Union(vec![TsType::String, TsType::Boolean]),
        );

        let error = validate(&schema, &json!(1.0), SchemaValidationOptions::default())
            .expect_err("a number matches neither member");

        assert_eq!(
            error,
            "$: value did not match any union member ($: expected string, got number; $: expected boolean, got number)"
        );
    }

    #[test]
    fn uint8_array_accepts_an_array_of_numbers_and_names_the_item_that_is_not_one() {
        let bytes = Schema::typed("Bytes", TsType::Uint8Array);
        let options = SchemaValidationOptions::default();

        assert!(validate(&bytes, &json!([1, 2]), options).is_ok());

        let error =
            validate(&bytes, &json!([1, "2"]), options).expect_err("a string is not a byte");
        assert_eq!(error, "$[1]: expected number, got string");
    }

    #[test]
    fn numeric_record_keys_must_be_canonical_integers() {
        let schema = Schema::typed(
            "Scores",
            TsType::Record {
                key: TsRecordKey::Number,
                value: Box::new(TsType::String),
            },
        );
        let options = SchemaValidationOptions::default();

        assert!(
            validate(
                &schema,
                &json!({ "42": "ok", "-7": "ok", "0": "ok" }),
                options
            )
            .is_ok()
        );

        // The decoder reads these keys as integers and refuses each of them.
        for key in ["NaN", "1.5", "1e3", "+42", "-0", "007"] {
            let error = validate(&schema, &json!({ key: "bad" }), options)
                .expect_err("a key the decoder refuses should fail");

            assert_eq!(
                error,
                format!("$: expected numeric record key, got {key:?}")
            );
        }
    }

    #[test]
    fn two_dependencies_with_one_name_must_have_the_same_shape() {
        let item = |ty| Schema::typed("Item", ty);
        let root = |dependencies| {
            Schema::typed("Root", TsType::Object(Vec::new())).with_dependencies(dependencies)
        };
        let check =
            |schema: &Schema| validate(schema, &json!({}), SchemaValidationOptions::default());

        assert!(check(&root(vec![item(TsType::Number), item(TsType::Number)])).is_ok());

        let error = check(&root(vec![item(TsType::Number), item(TsType::String)]))
            .expect_err("two shapes cannot share a name");
        assert_eq!(
            error,
            "two schemas are named `Item` but have different shapes"
        );
    }

    #[test]
    fn validates_resolved_type_ref_dependencies() {
        let schema = Schema::typed(
            "Input",
            TsType::Object(vec![TsField::required(
                "user_id",
                TsType::TypeRef(String::from("UserId")),
            )]),
        )
        .with_dependencies(vec![Schema::typed("UserId", TsType::Number)]);
        let options = SchemaValidationOptions::default();

        assert!(validate(&schema, &json!({ "user_id": 42 }), options).is_ok());

        let error = validate(&schema, &json!({ "user_id": "42" }), options)
            .expect_err("resolved TypeRef should validate against its dependency");

        assert_eq!(error, "$.user_id: expected number, got string");
    }

    #[test]
    fn unresolved_type_ref_remains_permissive_for_external_aliases() {
        let schema = Schema::typed("Input", TsType::TypeRef(String::from("ExternalAlias")));

        assert!(
            validate(
                &schema,
                &json!({ "anything": true }),
                SchemaValidationOptions::default(),
            )
            .is_ok()
        );
    }

    #[test]
    fn cyclic_type_ref_dependencies_do_not_recurse_forever() {
        let schema =
            Schema::typed("Input", TsType::TypeRef(String::from("Alias"))).with_dependencies(vec![
                Schema::typed("Alias", TsType::TypeRef(String::from("Alias"))),
            ]);

        assert!(
            validate(
                &schema,
                &json!("anything"),
                SchemaValidationOptions::default()
            )
            .is_ok()
        );
    }

    #[test]
    fn recursive_object_type_ref_dependencies_do_not_recurse_forever() {
        let schema = Schema::typed("Input", TsType::TypeRef(String::from("Node")))
            .with_dependencies(vec![Schema::typed(
                "Node",
                TsType::Object(vec![TsField::optional(
                    "child",
                    TsType::TypeRef(String::from("Node")),
                )]),
            )]);

        assert!(
            validate(
                &schema,
                &json!({ "child": { "child": "not validated recursively" } }),
                SchemaValidationOptions::default(),
            )
            .is_ok()
        );
    }
}
