//! Compile-time checks: shapes TsSchema cannot describe, and serde attributes the native
//! codec cannot mirror without an explicit `#[manivelle(codec = "json")]` opt-in.

use syn::{DeriveInput, Error, Fields, GenericArgument, PathArguments, Result, Type};

use crate::attrs::{JsonOnly, SchemaOnly};
use crate::directions::Directions;
use crate::model::{Body, Container, Field, Shape, Style, Tagging};

const TYPE_OPT_IN: &str = "add `#[manivelle(codec = \"json\")]` to the type";
const FIELD_OPT_IN: &str = "add `#[manivelle(codec = \"json\")]` (or a native `#[manivelle(with = \"...\")]`) to this field, or `#[manivelle(codec = \"json\")]` to the type";

pub(crate) fn check(container: &Container<'_>, input: &DeriveInput) -> Result<()> {
    check_transparent(container, input)?;
    for shape in shapes(container) {
        check_flatten_placement(shape)?;
        check_flatten_types(shape)?;
        check_field_codecs(shape)?;
    }
    check_internal_variants(container)?;
    if container.codecs() == Directions::NONE {
        return Ok(());
    }
    check_schema_only_attrs(container)?;
    if container.attrs.json_codec {
        return Ok(());
    }
    check_json_only_attrs(container)?;
    check_native_limits(container, input)
}

fn shapes<'c>(container: &'c Container<'_>) -> Vec<&'c Shape<'c>> {
    match &container.body {
        Body::Struct(shape) => vec![shape],
        Body::Enum(variants) => variants.iter().map(|variant| &variant.shape).collect(),
    }
}

fn check_transparent(container: &Container<'_>, input: &DeriveInput) -> Result<()> {
    if !container.attrs.transparent {
        return Ok(());
    }
    let Body::Struct(shape) = &container.body else {
        return Err(Error::new_spanned(
            input,
            "serde transparent is only supported for structs",
        ));
    };
    if container.transparent_field().is_some() {
        return Ok(());
    }
    let message = "serde transparent struct must contain exactly one field";
    match shape.original {
        Fields::Unit => Err(Error::new_spanned(input, message)),
        fields => Err(Error::new_spanned(fields, message)),
    }
}

fn check_flatten_placement(shape: &Shape<'_>) -> Result<()> {
    if shape.style == Style::Struct {
        return Ok(());
    }
    match shape.fields.iter().find(|field| field.attrs.flatten) {
        Some(field) => Err(Error::new_spanned(
            field.original,
            "serde flatten is only supported on named struct fields",
        )),
        None => Ok(()),
    }
}

/// Rejects flattened fields whose type is syntactically never an object or a string-keyed
/// map; other types are checked when the schema is built.
fn check_flatten_types(shape: &Shape<'_>) -> Result<()> {
    shape
        .fields
        .iter()
        .filter(|field| field.attrs.flatten)
        .map(|field| field.option_inner().unwrap_or(field.ty))
        .try_for_each(|ty| check_mergeable(ty, "serde flatten requires a struct or map type"))
}

/// Rejects `ty`, which serde merges into the surrounding object (a flattened field or an
/// internally tagged newtype payload), when it is syntactically never an object or a
/// string-keyed map.
fn check_mergeable(ty: &Type, requirement: &str) -> Result<()> {
    if is_never_object(ty) {
        return Err(Error::new_spanned(ty, requirement));
    }
    match numeric_map_key(ty) {
        Some(key) => Err(Error::new_spanned(
            key,
            "serde cannot merge a map with numeric keys into an object; use a string-keyed map",
        )),
        None => Ok(()),
    }
}

const INTEGER_TYPES: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize",
];

/// Scalars, strings, sequences, non-empty tuples and arrays: values serde cannot flatten.
fn is_never_object(ty: &Type) -> bool {
    const NON_OBJECT_TYPES: &[&str] = &[
        "bool", "char", "str", "String", "f32", "f64", "Vec", "VecDeque", "HashSet", "BTreeSet",
    ];
    match peel(ty) {
        Type::Array(_) | Type::Slice(_) => true,
        Type::Tuple(tuple) => !tuple.elems.is_empty(),
        Type::Path(path) if path.qself.is_none() => {
            path.path.segments.last().is_some_and(|segment| {
                let name = segment.ident.to_string();
                NON_OBJECT_TYPES.contains(&name.as_str()) || INTEGER_TYPES.contains(&name.as_str())
            })
        }
        _ => false,
    }
}

/// The key type of a `HashMap` or `BTreeMap` written with an integer key.
fn numeric_map_key(ty: &Type) -> Option<&Type> {
    let Type::Path(path) = peel(ty) else {
        return None;
    };
    let segment = path.path.segments.last()?;
    if path.qself.is_some() || !matches!(segment.ident.to_string().as_str(), "HashMap" | "BTreeMap")
    {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    let key = arguments.args.iter().find_map(|argument| match argument {
        GenericArgument::Type(key) => Some(key),
        _ => None,
    })?;
    is_integer(key).then_some(key)
}

fn is_integer(ty: &Type) -> bool {
    match peel(ty) {
        Type::Path(path) if path.qself.is_none() => path
            .path
            .get_ident()
            .is_some_and(|ident| INTEGER_TYPES.contains(&ident.to_string().as_str())),
        _ => false,
    }
}

/// `ty` without references, parentheses and invisible groups.
fn peel(ty: &Type) -> &Type {
    match ty {
        Type::Reference(reference) => peel(&reference.elem),
        Type::Paren(inner) => peel(&inner.elem),
        Type::Group(inner) => peel(&inner.elem),
        _ => ty,
    }
}

/// Internally tagged variants merge their payload into the tag's object: tuple variants
/// cannot, newtype payloads must be structs or string-keyed maps.
fn check_internal_variants(container: &Container<'_>) -> Result<()> {
    let (Tagging::Internal { .. }, Body::Enum(variants)) = (container.tagging(), &container.body)
    else {
        return Ok(());
    };
    for variant in variants {
        match variant.shape.style {
            Style::Tuple => {
                return Err(Error::new_spanned(
                    variant.original,
                    "serde internally tagged tuple enum variants are not supported",
                ));
            }
            Style::Newtype => check_mergeable(
                variant.shape.fields[0].ty,
                "serde internally tagged newtype variants require a struct or map type",
            )?,
            Style::Unit | Style::Struct => {}
        }
    }
    Ok(())
}

fn check_field_codecs(shape: &Shape<'_>) -> Result<()> {
    match shape
        .fields
        .iter()
        .find(|field| field.attrs.json_codec && field.attrs.with.is_some())
    {
        Some(field) => Err(Error::new_spanned(
            field.original,
            "`#[manivelle(with)]` and `#[manivelle(codec = \"json\")]` both choose this field's codec; keep one",
        )),
        None => Ok(()),
    }
}

/// `#[manivelle(rename/optional)]` would make the schema disagree with the codecs.
fn check_schema_only_attrs(container: &Container<'_>) -> Result<()> {
    let fields = shapes(container)
        .into_iter()
        .flat_map(|shape| &shape.fields)
        .flat_map(|field| &field.attrs.schema_only);
    let variants = match &container.body {
        Body::Enum(variants) => variants.as_slice(),
        Body::Struct(_) => &[],
    }
    .iter()
    .flat_map(|variant| &variant.attrs.schema_only);
    match fields.chain(variants).next() {
        Some(attr) => Err(schema_only_error(attr)),
        None => Ok(()),
    }
}

fn schema_only_error(attr: &SchemaOnly) -> Error {
    let key = path_text(&attr.path);
    Error::new_spanned(
        &attr.path,
        format!(
            "`#[manivelle({key})]` changes only the TypeScript schema, so the codec would disagree; use `#[serde({})]`, or mark the type `#[manivelle(schema_only)]`",
            attr.serde_equivalent
        ),
    )
}

fn check_json_only_attrs(container: &Container<'_>) -> Result<()> {
    let codecs = container.codecs();
    first_json_only(&container.attrs.json_only, codecs, TYPE_OPT_IN)?;
    if let Body::Enum(variants) = &container.body {
        for variant in variants {
            first_json_only(&variant.attrs.json_only, codecs, TYPE_OPT_IN)?;
        }
    }
    for shape in shapes(container) {
        for field in shape.fields.iter().filter(|field| !has_field_codec(field)) {
            first_json_only(&field.attrs.json_only, codecs, FIELD_OPT_IN)?;
        }
    }
    Ok(())
}

fn has_field_codec(field: &Field<'_>) -> bool {
    field.attrs.json_codec || field.attrs.with.is_some()
}

fn first_json_only(attrs: &[JsonOnly], codecs: Directions, opt_in: &str) -> Result<()> {
    match attrs.iter().find(|attr| attr.directions.intersects(codecs)) {
        Some(attr) => Err(Error::new_spanned(
            &attr.path,
            format!(
                "`#[serde({})]` needs the serde_json reference codec: {opt_in}",
                path_text(&attr.path)
            ),
        )),
        None => Ok(()),
    }
}

/// Serde features whose wire format the native codec does not reproduce.
fn check_native_limits(container: &Container<'_>, input: &DeriveInput) -> Result<()> {
    let codecs = container.codecs();
    let tagged = container.attrs.tag.is_some() || container.attrs.untagged;
    if tagged && matches!(container.body, Body::Struct(_)) {
        return Err(native_limit(&input.ident, "serde tag/untagged on a struct"));
    }
    for shape in shapes(container) {
        check_shape_limits(shape, codecs, container.attrs.deny_unknown_fields)?;
    }
    Ok(())
}

fn check_shape_limits(shape: &Shape<'_>, codecs: Directions, deny_unknown: bool) -> Result<()> {
    for field in &shape.fields {
        let limited = match shape.style {
            Style::Newtype => newtype_field_limits(field),
            Style::Tuple => tuple_field_limits(field),
            Style::Struct if deny_unknown && field.attrs.flatten => Directions::DECODE,
            Style::Struct | Style::Unit => Directions::NONE,
        };
        if limited.intersects(codecs) {
            return Err(native_limit(
                field.original,
                "this serde field attribute combination",
            ));
        }
    }
    Ok(())
}

/// A newtype is its inner value, so it has nothing to skip or default.
fn newtype_field_limits(field: &Field<'_>) -> Directions {
    let attrs = &field.attrs;
    let mut limited = Directions::NONE;
    if attrs.skip_serializing || attrs.skip_serializing_if.is_some() {
        limited |= Directions::ENCODE;
    }
    if attrs.skip_deserializing || attrs.default.is_some() {
        limited |= Directions::DECODE;
    }
    limited
}

/// Conditionally skipped or defaulted tuple elements shift array positions at runtime.
fn tuple_field_limits(field: &Field<'_>) -> Directions {
    let mut limited = Directions::NONE;
    if field.attrs.skip_serializing_if.is_some() {
        limited |= Directions::ENCODE;
    }
    if field.attrs.default.is_some() && !field.attrs.skip_deserializing {
        limited |= Directions::DECODE;
    }
    limited
}

fn native_limit(tokens: impl quote::ToTokens, what: &str) -> Error {
    Error::new_spanned(
        tokens,
        format!("{what} is not supported by the native codec; {TYPE_OPT_IN}"),
    )
}

fn path_text(path: &syn::Path) -> String {
    path.get_ident()
        .map_or_else(|| quote::quote!(#path).to_string(), ToString::to_string)
}
