//! Structural conversion between QuickJS values and `serde_json` values, without JSON
//! text. Untyped host boundaries use it directly; `serde_json::Value` codecs wrap it.

use rquickjs::atom::PredefinedAtom;
use rquickjs::{Array, Ctx, Filter, FromJs, IntoJs, Object, String as JsString, Value as JsValue};
use serde_json::{Map, Number, Value};

use super::JsEncode;
use super::codec::{
    CodecError, PathSegment, array_length, cautious_capacity, define_property, exact_integer,
    is_dropped_by_json, with_engine_bytes,
};
use super::validation::Snapshot;

/// How deeply arrays and objects may nest in a value that crosses into Rust as JSON: the
/// limit `serde_json` puts on parsing. A script controls its values, and this walk
/// recurses on the Rust stack, which QuickJS's stack limit does not cover: a cyclic array
/// or a 100 000 level nesting would otherwise overflow the host's stack and abort it.
const MAX_JSON_DEPTH: usize = 128;

/// Converts one QuickJS value into a JSON value.
///
/// Integral numbers within the JavaScript safe range become integer JSON numbers, as
/// `JSON.stringify` followed by `serde_json` would produce. Non-finite numbers fail.
pub(crate) fn js_to_json<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Value, CodecError> {
    json_value(ctx, value, 0)
}

/// Converts one QuickJS value into the snapshot contract validation checks.
///
/// Unlike [`js_to_json`], it follows what the native codecs accept rather than what
/// `JSON.stringify` writes: a `Uint8Array` or `ArrayBuffer` becomes its bytes, as
/// `NativeBytes` reads them, and `NaN` or `±Infinity` stay numbers.
pub(crate) fn js_to_snapshot<'js>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
) -> Result<Snapshot, CodecError> {
    snapshot_value(ctx, value, 0)
}

fn too_deep(value: &JsValue<'_>) -> CodecError {
    CodecError::from_js(
        value.type_name(),
        "json",
        "value is nested too deeply or contains a cycle",
    )
}

fn unsupported(value: &JsValue<'_>) -> CodecError {
    CodecError::from_js(value.type_name(), "json", "unsupported host bridge value")
}

/// Refuses what `JSON.stringify` would not write as itself: an object with a `toJSON`
/// method (a `Date`, say) and a `Map` or a `Set`, which it writes as `{}`. The error
/// reads as the one of a saved state that is not plain data.
fn reject_non_plain(object: &Object<'_>) -> Result<(), CodecError> {
    let mut plain = !object.contains_key(PredefinedAtom::ToJSON)?;
    if plain {
        let tag: JsValue<'_> = object.get(PredefinedAtom::SymbolToStringTag)?;
        if let Some(tag) = tag.as_string() {
            plain = !matches!(tag.to_string()?.as_str(), "Map" | "Set");
        }
    }
    if plain {
        return Ok(());
    }
    let constructor: JsValue<'_> = object.get(PredefinedAtom::Constructor)?;
    let name = match constructor.as_object() {
        Some(constructor) => {
            let name: JsValue<'_> = constructor.get(PredefinedAtom::Name)?;
            name.as_string().map(JsString::to_string).transpose()?
        }
        None => None,
    };
    Err(CodecError::from_js(
        "object",
        "json",
        format!(
            "{} is not plain data",
            name.as_deref().unwrap_or("an object")
        ),
    ))
}

fn json_value<'js>(ctx: &Ctx<'js>, value: JsValue<'js>, depth: usize) -> Result<Value, CodecError> {
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
        return float_to_json(value);
    }
    if value.is_string() {
        return Ok(Value::String(String::from_js(ctx, value)?));
    }
    if value.is_array() {
        if depth >= MAX_JSON_DEPTH {
            return Err(too_deep(&value));
        }
        return json_array(ctx, &Array::from_js(ctx, value)?, depth + 1);
    }
    if value.is_object() {
        if depth >= MAX_JSON_DEPTH {
            return Err(too_deep(&value));
        }
        let object = Object::from_js(ctx, value)?;
        reject_non_plain(&object)?;
        return json_object(ctx, &object, depth + 1);
    }
    Err(unsupported(&value))
}

fn json_array<'js>(ctx: &Ctx<'js>, array: &Array<'js>, depth: usize) -> Result<Value, CodecError> {
    let len = array_length(array, "json")?;
    let mut items = Vec::with_capacity(cautious_capacity::<Value>(len));
    for index in 0..len {
        let item: JsValue<'js> = array.get(index)?;
        let item = if is_dropped_by_json(&item) {
            Value::Null
        } else {
            json_value(ctx, item, depth).map_err(|error| error.at(index))?
        };
        items.push(item);
    }
    Ok(Value::Array(items))
}

fn json_object<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    depth: usize,
) -> Result<Value, CodecError> {
    let mut map = Map::new();
    for property in object.own_props::<JsString<'js>, JsValue<'js>>(Filter::default()) {
        let (key, value) = property?;
        if is_dropped_by_json(&value) {
            continue;
        }
        let key = key.to_string()?;
        let value = json_value(ctx, value, depth)
            .map_err(|error| error.at(PathSegment::Key(key.clone())))?;
        map.insert(key, value);
    }
    Ok(Value::Object(map))
}

fn float_to_json(value: f64) -> Result<Value, CodecError> {
    if let Some(integer) = exact_integer(value) {
        return Ok(Value::Number(Number::from(integer)));
    }
    Number::from_f64(value)
        .map(Value::Number)
        .ok_or_else(|| CodecError::from_js("number", "json", "non-finite number"))
}

fn snapshot_value<'js>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
    depth: usize,
) -> Result<Snapshot, CodecError> {
    if value.is_null() || value.is_undefined() {
        return Ok(Snapshot::Null);
    }
    if let Some(value) = value.as_bool() {
        return Ok(Snapshot::Bool(value));
    }
    if let Some(value) = value.as_int() {
        return Ok(Snapshot::Number(f64::from(value)));
    }
    if let Some(value) = value.as_float() {
        return Ok(Snapshot::Number(value));
    }
    if value.is_string() {
        return Ok(Snapshot::String(String::from_js(ctx, value)?));
    }
    if value.is_array() {
        if depth >= MAX_JSON_DEPTH {
            return Err(too_deep(&value));
        }
        return snapshot_array(ctx, &Array::from_js(ctx, value)?, depth + 1);
    }
    if value.is_object() {
        if depth >= MAX_JSON_DEPTH {
            return Err(too_deep(&value));
        }
        let object = Object::from_js(ctx, value)?;
        // A detached buffer has no bytes, as decoding reports it.
        if let Some(bytes) = with_engine_bytes(&object, |bytes| Box::<[u8]>::from(bytes)) {
            return Ok(Snapshot::Bytes(bytes.unwrap_or_default()));
        }
        return snapshot_object(ctx, &object, depth + 1);
    }
    Err(unsupported(&value))
}

fn snapshot_array<'js>(
    ctx: &Ctx<'js>,
    array: &Array<'js>,
    depth: usize,
) -> Result<Snapshot, CodecError> {
    let len = array_length(array, "json")?;
    let mut items = Vec::with_capacity(cautious_capacity::<Snapshot>(len));
    for index in 0..len {
        let item: JsValue<'js> = array.get(index)?;
        let item = if is_dropped_by_json(&item) {
            Snapshot::Null
        } else {
            snapshot_value(ctx, item, depth).map_err(|error| error.at(index))?
        };
        items.push(item);
    }
    Ok(Snapshot::Array(items))
}

fn snapshot_object<'js>(
    ctx: &Ctx<'js>,
    object: &Object<'js>,
    depth: usize,
) -> Result<Snapshot, CodecError> {
    let mut entries = Vec::new();
    for property in object.own_props::<JsString<'js>, JsValue<'js>>(Filter::default()) {
        let (key, value) = property?;
        if is_dropped_by_json(&value) {
            continue;
        }
        let key = key.to_string()?;
        let value = snapshot_value(ctx, value, depth)
            .map_err(|error| error.at(PathSegment::Key(key.clone())))?;
        entries.push((key, value));
    }
    Ok(Snapshot::Object(entries))
}

/// Converts one JSON value into a QuickJS value. Object keys become own data
/// properties, as `JSON.parse` defines them: a `"__proto__"` key never sets the prototype.
pub(crate) fn json_to_js<'js>(ctx: &Ctx<'js>, value: &Value) -> Result<JsValue<'js>, CodecError> {
    match value {
        Value::Null => Ok(JsValue::new_null(ctx.clone())),
        Value::Bool(value) => Ok(value.into_js(ctx)?),
        Value::Number(value) => number_to_js(ctx, value),
        Value::String(value) => Ok(value.as_str().into_js(ctx)?),
        Value::Array(values) => json_array_to_js(ctx, values),
        Value::Object(values) => {
            let object = Object::new(ctx.clone())?;
            for (key, value) in values {
                define_property(&object, key.as_str(), json_to_js(ctx, value)?)?;
            }
            Ok(object.into_value())
        }
    }
}

fn json_array_to_js<'js>(ctx: &Ctx<'js>, values: &[Value]) -> Result<JsValue<'js>, CodecError> {
    let array = Array::new(ctx.clone())?;
    for (index, value) in values.iter().enumerate() {
        array.set(index, json_to_js(ctx, value)?)?;
    }
    Ok(array.into_object().into_value())
}

/// Integer numbers follow the integer codecs, so values outside the safe range fail
/// instead of rounding.
fn number_to_js<'js>(ctx: &Ctx<'js>, value: &Number) -> Result<JsValue<'js>, CodecError> {
    if let Some(value) = value.as_i64() {
        return value.encode_js(ctx);
    }
    if let Some(value) = value.as_u64() {
        return value.encode_js(ctx);
    }
    value
        .as_f64()
        .ok_or_else(|| CodecError::from_js("json number", "number", "invalid number"))?
        .encode_js(ctx)
}
