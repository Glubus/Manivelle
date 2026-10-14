//! Structural conversion between QuickJS values and `serde_json` values, without JSON
//! text. Untyped host boundaries use it directly; `serde_json::Value` codecs wrap it.

use rquickjs::{
    Array, Ctx, Error as JsError, Filter, FromJs, IntoJs, Object, Result as JsResult,
    String as JsString, Value as JsValue,
};
use serde_json::{Map, Number, Value};

use super::JsEncode;
use super::codec::{
    array_length, at_index, at_path, cautious_capacity, define_property, exact_integer,
};

/// Stand-ins for the non-finite numbers in a validation snapshot, where `serde_json`
/// numbers must be finite. No JavaScript number snapshots to an unsigned integer above
/// the safe range (such numbers stay floats), so a stand-in never collides with a value.
const NAN_STAND_IN: u64 = u64::MAX;
const INFINITY_STAND_IN: u64 = u64::MAX - 1;
const NEG_INFINITY_STAND_IN: u64 = u64::MAX - 2;

/// What a conversion produces.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Purpose {
    /// A `serde_json::Value`, as `JSON.stringify` followed by `serde_json` reads it.
    Json,
    /// A snapshot for contract validation, shaped like what the native codecs read.
    Validation,
}

/// Converts one QuickJS value into a JSON value.
///
/// Integral numbers within the JavaScript safe range become integer JSON numbers, as
/// `JSON.stringify` followed by `serde_json` would produce. Non-finite numbers fail.
pub(crate) fn js_value_to_json<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> JsResult<Value> {
    to_json(ctx, value, Purpose::Json, 0)
}

/// Converts one QuickJS value into the snapshot contract validation checks.
///
/// Unlike [`js_value_to_json`], it follows what the native codecs accept rather than what
/// `JSON.stringify` writes: a `Uint8Array` or `ArrayBuffer` becomes the array of its
/// bytes, as `NativeBytes` reads it, and `NaN` or `±Infinity` becomes a stand-in number
/// that [`snapshot_number`] maps back.
pub(crate) fn js_value_to_validation_snapshot<'js>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
) -> JsResult<Value> {
    to_json(ctx, value, Purpose::Validation, 0)
}

/// The JavaScript number a snapshot number stands for, non-finite stand-ins included.
pub(crate) fn snapshot_number(number: &Number) -> Option<f64> {
    match number.as_u64() {
        Some(NAN_STAND_IN) => Some(f64::NAN),
        Some(INFINITY_STAND_IN) => Some(f64::INFINITY),
        Some(NEG_INFINITY_STAND_IN) => Some(f64::NEG_INFINITY),
        _ => number.as_f64(),
    }
}

/// How deeply arrays and objects may nest in a value that crosses into Rust as JSON: the
/// limit `serde_json` puts on parsing. A script controls its values, and this walk
/// recurses on the Rust stack, which QuickJS's stack limit does not cover: a cyclic array
/// or a 100 000 level nesting would otherwise overflow the host's stack and abort it.
const MAX_JSON_DEPTH: usize = 128;

fn too_deep(value: &JsValue<'_>) -> JsError {
    JsError::new_from_js_message(
        value.type_name(),
        "json",
        "value is nested too deeply or contains a cycle",
    )
}

fn to_json<'js>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
    purpose: Purpose,
    depth: usize,
) -> JsResult<Value> {
    if value.is_null() || value.is_undefined() {
        return Ok(Value::Null);
    }
    if let Some(value) = value.as_bool() {
        return Ok(Value::Bool(value));
    }
    if let Some(value) = value.as_int() {
        return Ok(Value::Number(Number::from(value)));
    }
    if let Some(value) = value.as_float() {
        return float_to_json(value, purpose);
    }
    if value.is_string() {
        return String::from_js(ctx, value).map(Value::String);
    }
    if value.is_array() {
        if depth >= MAX_JSON_DEPTH {
            return Err(too_deep(&value));
        }
        return array_to_json(ctx, &Array::from_js(ctx, value)?, purpose, depth + 1);
    }
    if value.is_object() {
        if depth >= MAX_JSON_DEPTH {
            return Err(too_deep(&value));
        }
        let object = Object::from_js(ctx, value)?;
        if purpose == Purpose::Validation
            && let Some(bytes) = bytes_snapshot(&object)
        {
            return Ok(bytes);
        }
        return object_to_json(ctx, &object, purpose, depth + 1);
    }
    Err(JsError::new_from_js_message(
        value.type_name(),
        "json",
        "unsupported host bridge value",
    ))
}

fn array_to_json<'js>(
    ctx: &Ctx<'js>,
    array: &Array<'js>,
    purpose: Purpose,
    depth: usize,
) -> JsResult<Value> {
    let len = array_length(array, "json")?;
    let mut items = Vec::with_capacity(cautious_capacity::<Value>(len));
    for index in 0..len {
        let item = to_json(ctx, array.get(index)?, purpose, depth)
            .map_err(|error| at_index(error, index))?;
        items.push(item);
    }
    Ok(Value::Array(items))
}

fn object_to_json<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    purpose: Purpose,
    depth: usize,
) -> JsResult<Value> {
    let mut map = Map::new();
    for property in object.own_props::<JsString<'js>, JsValue<'js>>(Filter::default()) {
        let (key, value) = property?;
        let key = key.to_string()?;
        let value = to_json(ctx, value, purpose, depth)
            .map_err(|error| at_path(error, format_args!("[{key}]")))?;
        map.insert(key, value);
    }
    Ok(Value::Object(map))
}

/// The bytes of a `Uint8Array` or `ArrayBuffer` as an array of numbers, `None` for any
/// other object. A detached buffer snapshots as no bytes; decoding reports it.
fn bytes_snapshot(object: &Object<'_>) -> Option<Value> {
    let bytes = if let Some(view) = object.as_typed_array::<u8>() {
        // SAFETY: the bytes are copied before any JavaScript can run again.
        unsafe { view.as_bytes() }
    } else if let Some(buffer) = object.as_array_buffer() {
        // SAFETY: the bytes are copied before any JavaScript can run again.
        unsafe { buffer.as_bytes() }
    } else {
        return None;
    };
    let items = bytes
        .unwrap_or_default()
        .iter()
        .map(|byte| Value::Number(Number::from(*byte)))
        .collect();
    Some(Value::Array(items))
}

fn float_to_json(value: f64, purpose: Purpose) -> JsResult<Value> {
    if let Some(integer) = exact_integer(value) {
        return Ok(Value::Number(Number::from(integer)));
    }
    if let Some(number) = Number::from_f64(value) {
        return Ok(Value::Number(number));
    }
    match purpose {
        Purpose::Json => Err(JsError::new_from_js_message(
            "number",
            "json",
            "non-finite number",
        )),
        Purpose::Validation => Ok(Value::Number(Number::from(non_finite_stand_in(value)))),
    }
}

fn non_finite_stand_in(value: f64) -> u64 {
    if value.is_nan() {
        NAN_STAND_IN
    } else if value.is_sign_positive() {
        INFINITY_STAND_IN
    } else {
        NEG_INFINITY_STAND_IN
    }
}

/// Converts one JSON value into a QuickJS value. Object keys become own data
/// properties, as `JSON.parse` defines them: a `"__proto__"` key never sets the prototype.
pub(crate) fn json_to_js_value<'js>(ctx: &Ctx<'js>, value: &Value) -> JsResult<JsValue<'js>> {
    match value {
        Value::Null => Ok(JsValue::new_null(ctx.clone())),
        Value::Bool(value) => value.into_js(ctx),
        Value::Number(value) => number_to_js_value(ctx, value),
        Value::String(value) => value.as_str().into_js(ctx),
        Value::Array(values) => json_array_to_js_value(ctx, values),
        Value::Object(values) => {
            let object = Object::new(ctx.clone())?;
            for (key, value) in values {
                define_property(&object, key.as_str(), json_to_js_value(ctx, value)?)?;
            }
            Ok(object.into_value())
        }
    }
}

fn json_array_to_js_value<'js>(ctx: &Ctx<'js>, values: &[Value]) -> JsResult<JsValue<'js>> {
    let array = Array::new(ctx.clone())?;
    for (index, value) in values.iter().enumerate() {
        array.set(index, json_to_js_value(ctx, value)?)?;
    }
    Ok(array.into_object().into_value())
}

/// Integer numbers follow the integer codecs, so values outside the safe range fail
/// instead of rounding.
fn number_to_js_value<'js>(ctx: &Ctx<'js>, value: &Number) -> JsResult<JsValue<'js>> {
    if let Some(value) = value.as_i64() {
        return value.encode_js(ctx);
    }
    if let Some(value) = value.as_u64() {
        return value.encode_js(ctx);
    }
    value
        .as_f64()
        .ok_or_else(|| JsError::new_from_js_message("json number", "number", "invalid number"))?
        .encode_js(ctx)
}
