//! Runtime support for `#[derive(TsSchema)]` codecs.
//!
//! Only macro output calls these functions. They keep the generated code small and put
//! every derive-specific error message in one place.

use rquickjs::{
    Array, Ctx, Error as JsError, Object, Result as JsResult, String as JsString, Value as JsValue,
};
use serde::{Serialize, de::DeserializeOwned};

use super::field_atoms::define_field;
pub use super::js_text::JsText;
use super::{JsDecode, JsEncode, at_path, codec_error, define_property_named, is_dropped_by_json};

/// Encodes `value` through the serde_json reference path (`#[manivelle(codec = "json")]`).
pub fn json_encode<'js, T: Serialize + ?Sized>(
    value: &T,
    ctx: &Ctx<'js>,
) -> JsResult<JsValue<'js>> {
    json_encode_with(value, ctx, T::serialize)
}

/// Decodes `value` through the serde_json reference path (`#[manivelle(codec = "json")]`).
pub fn json_decode<'js, T: DeserializeOwned>(ctx: &Ctx<'js>, value: JsValue<'js>) -> JsResult<T> {
    json_decode_with(ctx, value, T::deserialize)
}

/// Encodes `value` through a serde `serialize_with` function into JSON, then into JS.
pub fn json_encode_with<'js, T: ?Sized>(
    value: &T,
    ctx: &Ctx<'js>,
    serialize: impl FnOnce(
        &T,
        serde_json::value::Serializer,
    ) -> Result<serde_json::Value, serde_json::Error>,
) -> JsResult<JsValue<'js>> {
    let json = serialize(value, serde_json::value::Serializer).map_err(|error| {
        JsError::new_into_js_message(std::any::type_name::<T>(), "json", error.to_string())
    })?;
    json.encode_js(ctx)
}

/// Decodes `value` into JSON, then through a serde `deserialize_with` function.
pub fn json_decode_with<'js, T>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
    deserialize: impl FnOnce(serde_json::Value) -> Result<T, serde_json::Error>,
) -> JsResult<T> {
    let json = serde_json::Value::decode_js(ctx, value)?;
    deserialize(json)
        .map_err(|error| codec_error("json", std::any::type_name::<T>(), error.to_string()))
}

/// Returns the first present property among `names` (a field name followed by its
/// aliases). Properties `JSON.stringify` drops (undefined, functions, symbols) count as
/// absent, so inherited methods such as `constructor` never shadow a missing field.
pub fn field_value<'js>(
    object: &Object<'js>,
    names: &[&'static str],
) -> JsResult<Option<JsValue<'js>>> {
    for name in names {
        let value: JsValue<'js> = object.get(*name)?;
        if !is_dropped_by_json(&value) {
            return Ok(Some(value));
        }
    }
    Ok(None)
}

/// Decodes a field without a default through `decode`. A missing field behaves like
/// serde's missing field: `Option<T>` becomes `None`, any other type fails with
/// `missing field` at `names[0]`.
pub fn decode_field<'js, T>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    names: &[&'static str],
    decode: impl FnOnce(&Ctx<'js>, JsValue<'js>) -> JsResult<T>,
) -> JsResult<T> {
    match field_value(object, names)? {
        Some(value) => decode(ctx, value).map_err(|error| at_path(error, names[0])),
        None => decode(ctx, JsValue::new_undefined(ctx.clone()))
            .map_err(|_| missing_field(names[0], std::any::type_name::<T>())),
    }
}

/// Decodes a field through `decode`, taking `default()` when it is missing
/// (`#[serde(default)]`).
pub fn decode_field_or<'js, T>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    names: &[&'static str],
    decode: impl FnOnce(&Ctx<'js>, JsValue<'js>) -> JsResult<T>,
    default: impl FnOnce() -> T,
) -> JsResult<T> {
    match field_value(object, names)? {
        Some(value) => decode(ctx, value).map_err(|error| at_path(error, names[0])),
        None => Ok(default()),
    }
}

/// Returns a property that must be present, such as adjacently tagged enum content.
pub fn require_field<'js>(
    object: &Object<'js>,
    name: &'static str,
    rust: &'static str,
) -> JsResult<JsValue<'js>> {
    field_value(object, &[name])?.ok_or_else(|| missing_field(name, rust))
}

fn missing_field(name: &'static str, rust: &'static str) -> JsError {
    at_path(codec_error("undefined", rust, "missing field"), name)
}

/// Encodes `value` through `encode` into `object[name]`, prefixing errors with `name`.
/// `object` is the plain object the derived encoder just created.
pub fn encode_field<'js, T: ?Sized>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    name: &'static str,
    value: &T,
    encode: impl FnOnce(&T, &Ctx<'js>) -> JsResult<JsValue<'js>>,
) -> JsResult<()> {
    let value = encode(value, ctx).map_err(|error| at_path(error, name))?;
    define_field(ctx, object, name, value)
}

/// Encodes `value` through `encode` into `array[index]`, prefixing errors with `[index]`.
pub fn encode_item<'js, T: ?Sized>(
    ctx: &Ctx<'js>,
    array: &Array<'js>,
    index: usize,
    value: &T,
    encode: impl FnOnce(&T, &Ctx<'js>) -> JsResult<JsValue<'js>>,
) -> JsResult<()> {
    let value = encode(value, ctx).map_err(|error| at_path(error, format_args!("[{index}]")))?;
    array.set(index, value)
}

/// Decodes `array[index]` through `decode`, prefixing errors with `[index]`.
pub fn decode_item<'js, T>(
    ctx: &Ctx<'js>,
    array: &Array<'js>,
    index: usize,
    decode: impl FnOnce(&Ctx<'js>, JsValue<'js>) -> JsResult<T>,
) -> JsResult<T> {
    let value: JsValue<'js> = array.get(index)?;
    decode(ctx, value).map_err(|error| at_path(error, format_args!("[{index}]")))
}

/// Copies the own enumerable properties of `source` into `target`, for
/// `#[serde(flatten)]` fields and internally tagged newtype variants. `null` and
/// `undefined` (a `None` or a unit value) add nothing, like serde.
pub fn merge_object<'js>(
    target: &Object<'js>,
    source: JsValue<'js>,
    rust: &'static str,
) -> JsResult<()> {
    if source.is_null() || source.is_undefined() {
        return Ok(());
    }
    let kind = source.type_name();
    let source = source
        .into_object()
        .filter(|object| !object.as_value().is_array())
        .ok_or_else(|| {
            JsError::new_into_js_message(
                kind,
                rust,
                format!("expected a struct or map to merge into {rust}, got {kind}"),
            )
        })?;
    for property in source.props::<JsString<'js>, JsValue<'js>>() {
        let (key, value) = property?;
        define_property_named(target, key.as_value(), value)?;
    }
    Ok(())
}

/// Copies `object` without the `excluded` keys. This is the input serde hands a flattened
/// field (keys not claimed by the outer struct) or an internally tagged newtype variant
/// (everything but the tag).
pub fn object_without<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    excluded: &[&'static str],
) -> JsResult<Object<'js>> {
    let rest = Object::new(ctx.clone())?;
    for property in object.props::<JsString<'js>, JsValue<'js>>() {
        let (key, value) = property?;
        if !excluded.contains(&JsText::new(key.clone(), "object key")?.as_str()) {
            define_property_named(&rest, key.as_value(), value)?;
        }
    }
    Ok(rest)
}

/// Fails with `unknown field` when `object` has an own key outside `known`
/// (`#[serde(deny_unknown_fields)]`).
pub fn reject_unknown_fields<'js>(
    object: &Object<'js>,
    known: &[&'static str],
    rust: &'static str,
) -> JsResult<()> {
    for key in object.keys::<JsString<'js>>() {
        let key = JsText::new(key?, rust)?;
        if !known.contains(&key.as_str()) {
            return Err(codec_error(
                "object",
                rust,
                format!(
                    "unknown field `{}`, expected {}",
                    key.as_str(),
                    one_of(known)
                ),
            ));
        }
    }
    Ok(())
}

/// Reads a variant name from a JS string without copying it into a Rust `String`.
pub fn variant_name<'js>(value: JsValue<'js>, rust: &'static str) -> JsResult<JsText<'js>> {
    let kind = value.type_name();
    match value.into_string() {
        Some(name) => JsText::new(name, rust),
        None => Err(codec_error(
            kind,
            rust,
            format!("expected variant name string, got {kind}"),
        )),
    }
}

/// Reads the variant name stored under `tag` (internally and adjacently tagged enums).
pub fn tag_field<'js>(
    object: &Object<'js>,
    tag: &'static str,
    rust: &'static str,
) -> JsResult<JsText<'js>> {
    let value = require_field(object, tag, rust)?;
    variant_name(value, rust).map_err(|error| at_path(error, tag))
}

/// Splits an externally tagged enum value into its variant name and payload. A string
/// names a variant without payload; an object must hold exactly one own key.
pub fn external_variant<'js>(
    value: JsValue<'js>,
    rust: &'static str,
) -> JsResult<(JsText<'js>, Option<JsValue<'js>>)> {
    if value.is_string() {
        return Ok((variant_name(value, rust)?, None));
    }
    let object = super::expect_object(value, rust)?;
    let mut properties = object.props::<JsString<'js>, JsValue<'js>>();
    match (properties.next().transpose()?, properties.next()) {
        (Some((name, payload)), None) => Ok((JsText::new(name, rust)?, Some(payload))),
        _ => Err(codec_error(
            "object",
            rust,
            "expected variant name string or object with exactly one key",
        )),
    }
}

/// Returns the payload of an externally tagged tuple, newtype or struct variant, which
/// cannot be written as a bare variant name.
pub fn variant_payload<'js>(
    payload: Option<JsValue<'js>>,
    rust: &'static str,
    variant: &'static str,
) -> JsResult<JsValue<'js>> {
    payload.ok_or_else(|| {
        codec_error(
            "string",
            rust,
            format!("expected payload for variant `{variant}`, got bare variant name"),
        )
    })
}

/// Checks the optional payload of a unit variant: absent, or a value `()` accepts.
pub fn unit_payload<'js>(ctx: &Ctx<'js>, payload: Option<JsValue<'js>>) -> JsResult<()> {
    match payload {
        Some(payload) => <() as JsDecode>::decode_js(ctx, payload),
        None => Ok(()),
    }
}

/// Error for a variant name that matches no variant.
pub fn unknown_variant(
    found: &str,
    rust: &'static str,
    variants: &'static [&'static str],
) -> JsError {
    codec_error(
        "string",
        rust,
        format!("unknown variant `{found}`, expected {}", one_of(variants)),
    )
}

/// Error for an untagged enum value that no variant accepts.
pub fn untagged_mismatch(value: &JsValue<'_>, rust: &'static str) -> JsError {
    codec_error(
        value.type_name(),
        rust,
        format!("data did not match any variant of untagged enum {rust}"),
    )
}

/// Error for encoding a `#[serde(skip_serializing)]` variant.
pub fn skipped_variant(rust: &'static str, variant: &'static str) -> JsError {
    JsError::new_into_js_message(
        rust,
        "value",
        format!("the enum variant {rust}::{variant} cannot be serialized"),
    )
}

fn one_of(names: &[&str]) -> String {
    match names {
        [] => String::from("nothing"),
        [name] => format!("`{name}`"),
        names => {
            let quoted: Vec<String> = names.iter().map(|name| format!("`{name}`")).collect();
            format!("one of {}", quoted.join(", "))
        }
    }
}
