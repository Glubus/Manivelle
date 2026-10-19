//! Runtime validation for host contract schemas.

use std::borrow::Cow;
use std::collections::HashMap;
use std::fmt;

use serde_json::Value;

use super::{Schema, TsEnumVariant, TsField, TsLiteral, TsRecordKey, TsType};

/// A value to validate, owned and shaped the way validation reads it. A JSON value converts
/// with `From<&Value>`; a QuickJS value converts in `bridge.rs`, which keeps a byte buffer as
/// its bytes rather than one value per byte.
#[derive(Clone, Debug)]
pub(crate) enum Snapshot {
    Null,
    Bool(bool),
    /// A JavaScript number, non-finite ones included.
    Number(f64),
    String(String),
    Array(Vec<Snapshot>),
    /// Entries in insertion order.
    Object(Vec<(String, Snapshot)>),
    /// The bytes of a `Uint8Array` or `ArrayBuffer`, validated as an array of numbers.
    Bytes(Box<[u8]>),
}

impl Snapshot {
    /// The kind `expected` messages name this value by.
    fn kind(&self) -> &'static str {
        match self {
            Self::Null => "null",
            Self::Bool(_) => "boolean",
            Self::Number(_) => "number",
            Self::String(_) => "string",
            Self::Array(_) | Self::Bytes(_) => "array",
            Self::Object(_) => "object",
        }
    }

    fn as_object(&self) -> Option<&[(String, Snapshot)]> {
        match self {
            Self::Object(entries) => Some(entries),
            _ => None,
        }
    }

    /// The elements of an array-like value. A byte buffer yields its bytes as numbers.
    fn elements(&self) -> Option<Box<dyn ExactSizeIterator<Item = Cow<'_, Snapshot>> + '_>> {
        match self {
            Self::Array(items) => Some(Box::new(items.iter().map(Cow::Borrowed))),
            Self::Bytes(bytes) => Some(Box::new(
                bytes
                    .iter()
                    .map(|byte| Cow::Owned(Self::Number(f64::from(*byte)))),
            )),
            _ => None,
        }
    }
}

impl From<&Value> for Snapshot {
    fn from(value: &Value) -> Self {
        match value {
            Value::Null => Self::Null,
            Value::Bool(value) => Self::Bool(*value),
            // `as_f64` fails only for arbitrary-precision numbers beyond the `f64` range.
            Value::Number(number) => Self::Number(number.as_f64().unwrap_or(f64::NAN)),
            Value::String(value) => Self::String(value.clone()),
            Value::Array(items) => Self::Array(items.iter().map(Self::from).collect()),
            Value::Object(entries) => Self::Object(
                entries
                    .iter()
                    .map(|(key, value)| (key.clone(), Self::from(value)))
                    .collect(),
            ),
        }
    }
}

pub(crate) fn validate_schema_with_options(
    schema: &Schema,
    value: &Snapshot,
    options: SchemaValidationOptions,
) -> Result<(), String> {
    let mut context = ValidationContext::new(schema);
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
    /// The dependencies being validated, so that a recursive reference stops.
    resolving: Vec<usize>,
}

impl<'a> ValidationContext<'a> {
    fn new(schema: &'a Schema) -> Self {
        let mut dependencies = HashMap::new();
        collect_dependencies(&schema.dependencies, &mut dependencies);
        Self {
            dependencies,
            resolving: Vec::new(),
        }
    }
}

fn collect_dependencies<'a>(
    schemas: &'a [Schema],
    dependencies: &mut HashMap<&'a str, Dependency<'a>>,
) {
    for schema in schemas {
        if let Some(name) = schema.declared_name()
            && !dependencies.contains_key(name)
        {
            let id = dependencies.len();
            dependencies.insert(name, Dependency { id, schema });
        }
        collect_dependencies(&schema.dependencies, dependencies);
    }
}

fn validate_type(
    ty: &TsType,
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    match ty {
        TsType::Unknown | TsType::Json => Ok(()),
        TsType::TypeRef(name) => validate_type_ref(name, value, path, options, context),
        TsType::Void | TsType::Null => validate_null(value, path),
        TsType::Boolean => validate_bool(value, path),
        TsType::Number => validate_number(value, path),
        TsType::String => validate_string(value, path),
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

fn validate_type_ref(
    name: &str,
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
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

fn validate_null(value: &Snapshot, path: &Path<'_>) -> Result<(), String> {
    matches!(value, Snapshot::Null)
        .then_some(())
        .ok_or_else(|| expected(path, "null", value))
}

fn validate_bool(value: &Snapshot, path: &Path<'_>) -> Result<(), String> {
    matches!(value, Snapshot::Bool(_))
        .then_some(())
        .ok_or_else(|| expected(path, "boolean", value))
}

fn validate_number(value: &Snapshot, path: &Path<'_>) -> Result<(), String> {
    matches!(value, Snapshot::Number(_))
        .then_some(())
        .ok_or_else(|| expected(path, "number", value))
}

fn validate_string(value: &Snapshot, path: &Path<'_>) -> Result<(), String> {
    matches!(value, Snapshot::String(_))
        .then_some(())
        .ok_or_else(|| expected(path, "string", value))
}

fn validate_literal(literal: &TsLiteral, value: &Snapshot, path: &Path<'_>) -> Result<(), String> {
    let valid = match (literal, value) {
        (TsLiteral::String(expected), Snapshot::String(value)) => value == expected,
        (TsLiteral::Number(expected), Snapshot::Number(value)) => {
            literal_number_matches(expected, *value)
        }
        (TsLiteral::Boolean(expected), Snapshot::Bool(value)) => value == expected,
        _ => false,
    };
    valid
        .then_some(())
        .ok_or_else(|| format!("{path}: expected literal {}", literal_label(literal)))
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

fn validate_object(
    shape: ObjectShape<'_>,
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    let Some(entries) = value.as_object() else {
        return Err(expected(path, "object", value));
    };

    if shape.rest.is_none() && options.reject_unknown_fields {
        reject_unknown_object_fields(shape, entries, path)?;
    }
    validate_declared_fields(shape.fields, entries, path, options, context)?;
    match shape.rest {
        Some(rest) => validate_rest_fields(shape, rest, entries, path, options, context),
        None => Ok(()),
    }
}

fn validate_declared_fields(
    fields: &[TsField],
    entries: &[(String, Snapshot)],
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    for field in fields {
        let field_path = Path::Field(path, field.name.as_str());
        match field_value(entries, &field.name) {
            Some(value) => validate_type(&field.ty, value, &field_path, options, context)?,
            None if field.optional => {}
            None => return Err(format!("{field_path}: missing required field")),
        }
    }
    Ok(())
}

/// Validates every undeclared key of an open object against its `rest` type.
fn validate_rest_fields(
    shape: ObjectShape<'_>,
    rest: &TsType,
    entries: &[(String, Snapshot)],
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    for (key, item) in entries.iter().filter(|(key, _)| shape.is_undeclared(key)) {
        validate_type(
            rest,
            item,
            &Path::Field(path, key.as_str()),
            options,
            context,
        )?;
    }
    Ok(())
}

fn reject_unknown_object_fields(
    shape: ObjectShape<'_>,
    entries: &[(String, Snapshot)],
    path: &Path<'_>,
) -> Result<(), String> {
    match entries.iter().find(|(key, _)| shape.is_undeclared(key)) {
        Some((key, _)) => Err(format!(
            "{}: unknown field",
            Path::Field(path, key.as_str())
        )),
        None => Ok(()),
    }
}

fn field_value<'e>(entries: &'e [(String, Snapshot)], name: &str) -> Option<&'e Snapshot> {
    entries
        .iter()
        .find(|(key, _)| key.as_str() == name)
        .map(|(_, value)| value)
}

fn validate_array(
    item: &TsType,
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    let Some(elements) = value.elements() else {
        return Err(expected(path, "array", value));
    };

    for (index, element) in elements.enumerate() {
        validate_type(item, &element, &Path::Index(path, index), options, context)?;
    }
    Ok(())
}

fn validate_tuple(
    items: &[TsType],
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    let Some(elements) = value.elements() else {
        return Err(expected(path, "tuple array", value));
    };
    if elements.len() != items.len() {
        return Err(format!(
            "{path}: expected tuple length {}, got {}",
            items.len(),
            elements.len()
        ));
    }

    for (index, (ty, element)) in items.iter().zip(elements).enumerate() {
        validate_type(ty, &element, &Path::Index(path, index), options, context)?;
    }
    Ok(())
}

fn validate_enum(
    tag: Option<&str>,
    variants: &[TsEnumVariant],
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    match tag {
        Some(tag) => validate_tagged_enum(tag, variants, value, path, options, context),
        None => validate_unit_enum(variants, value, path),
    }
}

fn validate_unit_enum(
    variants: &[TsEnumVariant],
    value: &Snapshot,
    path: &Path<'_>,
) -> Result<(), String> {
    let Snapshot::String(name) = value else {
        return Err(expected(path, "enum string literal", value));
    };
    variants
        .iter()
        .any(|variant| variant.name == *name)
        .then_some(())
        .ok_or_else(|| format!("{path}: unknown enum variant {name:?}"))
}

fn validate_tagged_enum(
    tag: &str,
    variants: &[TsEnumVariant],
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    let Some(entries) = value.as_object() else {
        return Err(expected(path, "tagged enum object", value));
    };
    let tag_path = Path::Field(path, tag);
    let Some(Snapshot::String(tag_value)) = field_value(entries, tag) else {
        return Err(format!("{tag_path}: missing enum tag string"));
    };
    let Some(variant) = variants
        .iter()
        .find(|variant| variant.name == tag_value.as_str())
    else {
        return Err(format!("{tag_path}: unknown enum variant {tag_value:?}"));
    };

    let allowed_extra_fields = [tag];
    let shape = ObjectShape::new(&variant.fields)
        .with_rest(variant.rest.as_deref())
        .with_allowed_extra_fields(&allowed_extra_fields);
    validate_object(shape, value, path, options, context)
}

fn validate_record(
    key: TsRecordKey,
    item: &TsType,
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    let Some(entries) = value.as_object() else {
        return Err(expected(path, "record object", value));
    };

    for (field, field_value) in entries {
        validate_record_key(key, field, path)?;
        validate_type(
            item,
            field_value,
            &Path::Field(path, field.as_str()),
            options,
            context,
        )?;
    }
    Ok(())
}

fn validate_record_key(key: TsRecordKey, field: &str, path: &Path<'_>) -> Result<(), String> {
    match key {
        TsRecordKey::String => Ok(()),
        TsRecordKey::Number => validate_numeric_record_key(field)
            .then_some(())
            .ok_or_else(|| format!("{path}: expected numeric record key, got {field:?}")),
    }
}

fn validate_numeric_record_key(field: &str) -> bool {
    field.parse::<f64>().is_ok_and(f64::is_finite)
}

/// Accepts a value that some member accepts. Otherwise the message keeps every member's
/// own error, so the mismatch is visible.
fn validate_union(
    types: &[TsType],
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    let mut member_errors = Vec::with_capacity(types.len());
    for ty in types {
        match validate_type(ty, value, path, options, context) {
            Ok(()) => return Ok(()),
            Err(error) => member_errors.push(error),
        }
    }

    Err(format!(
        "{path}: value did not match any union member ({})",
        member_errors.join("; ")
    ))
}

/// Accepts `null` for an optional or nullable type, and validates anything else as `inner`.
fn validate_nullish(
    inner: &TsType,
    value: &Snapshot,
    path: &Path<'_>,
    options: SchemaValidationOptions,
    context: &mut ValidationContext<'_>,
) -> Result<(), String> {
    if matches!(value, Snapshot::Null) {
        return Ok(());
    }
    validate_type(inner, value, path, options, context)
}

fn expected(path: &Path<'_>, expected: &str, value: &Snapshot) -> String {
    format!("{path}: expected {expected}, got {}", value.kind())
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn validates_nested_object_shape() {
        let schema = Schema::typed(
            "Input",
            TsType::Object(vec![
                TsField::required("id", TsType::Number),
                TsField::optional("tag", TsType::String),
            ]),
        );

        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "id": 1 })),
                SchemaValidationOptions::default(),
            )
            .is_ok()
        );
        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "tag": "x" })),
                SchemaValidationOptions::default(),
            )
            .is_err()
        );
        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "id": "1" })),
                SchemaValidationOptions::default(),
            )
            .is_err()
        );
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

        let error = validate_schema_with_options(
            &schema,
            &Snapshot::from(&json!({
                "id": 1,
                "profile": {
                    "name": "Ada",
                    "role": "admin"
                }
            })),
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

        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "type": "damage", "amount": 12 })),
                options,
            )
            .is_ok()
        );
        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "type": "damage", "amount": 12, "debug": true })),
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

        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!("ok")),
                SchemaValidationOptions::default()
            )
            .is_ok()
        );
        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!(42)),
                SchemaValidationOptions::default()
            )
            .is_ok()
        );
        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::Null,
                SchemaValidationOptions::default()
            )
            .is_ok()
        );
        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!(true)),
                SchemaValidationOptions::default()
            )
            .is_err()
        );
    }

    #[test]
    fn union_error_reports_each_member_mismatch() {
        let schema = Schema::typed(
            "Input",
            TsType::Union(vec![TsType::String, TsType::Boolean]),
        );

        let error = validate_schema_with_options(
            &schema,
            &Snapshot::Number(1.0),
            SchemaValidationOptions::default(),
        )
        .expect_err("a number matches neither member");

        assert_eq!(
            error,
            "$: value did not match any union member ($: expected string, got number; $: expected boolean, got number)"
        );
    }

    #[test]
    fn uint8_array_validates_byte_buffers_as_numbers() {
        let bytes = Schema::typed("Bytes", TsType::Uint8Array);
        assert!(
            validate_schema_with_options(
                &bytes,
                &Snapshot::Bytes(Box::from(&[1u8, 2][..])),
                SchemaValidationOptions::default(),
            )
            .is_ok()
        );

        let strings = Schema::typed("Strings", TsType::Array(Box::new(TsType::String)));
        let error = validate_schema_with_options(
            &strings,
            &Snapshot::Bytes(Box::from(&[1u8][..])),
            SchemaValidationOptions::default(),
        )
        .expect_err("bytes are numbers, not strings");

        assert_eq!(error, "$[0]: expected string, got number");
    }

    #[test]
    fn numeric_record_key_validation_rejects_non_finite_numbers() {
        let schema = Schema::typed(
            "Scores",
            TsType::Record {
                key: TsRecordKey::Number,
                value: Box::new(TsType::String),
            },
        );

        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "42": "ok", "-1.5": "ok", "1e3": "ok" })),
                SchemaValidationOptions::default(),
            )
            .is_ok()
        );

        let error = validate_schema_with_options(
            &schema,
            &Snapshot::from(&json!({ "NaN": "bad" })),
            SchemaValidationOptions::default(),
        )
        .expect_err("non-finite numeric record key should fail");

        assert_eq!(error, "$: expected numeric record key, got \"NaN\"");
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

        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "user_id": 42 })),
                SchemaValidationOptions::default(),
            )
            .is_ok()
        );

        let error = validate_schema_with_options(
            &schema,
            &Snapshot::from(&json!({ "user_id": "42" })),
            SchemaValidationOptions::default(),
        )
        .expect_err("resolved TypeRef should validate against its dependency");

        assert_eq!(error, "$.user_id: expected number, got string");
    }

    #[test]
    fn unresolved_type_ref_remains_permissive_for_external_aliases() {
        let schema = Schema::typed("Input", TsType::TypeRef(String::from("ExternalAlias")));

        assert!(
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "anything": true })),
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
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!("anything")),
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
            validate_schema_with_options(
                &schema,
                &Snapshot::from(&json!({ "child": { "child": "not validated recursively" } })),
                SchemaValidationOptions::default(),
            )
            .is_ok()
        );
    }
}
