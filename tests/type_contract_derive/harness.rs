//! Checks that the native codecs and `TsSchema` of a type agree with `serde_json`.

use std::fmt::Debug;

use manivelle::js::{Context, Ctx, Runtime, Value as JsValue};
use manivelle::{JsDecode, JsEncode, TsSchema};
use serde::Serialize;
use serde::de::DeserializeOwned;
use serde_json::{Number, Value};

pub(crate) fn with_js<R>(run: impl FnOnce(&Ctx<'_>) -> R) -> R {
    let runtime = Runtime::new().expect("create runtime");
    let context = Context::full(&runtime).expect("create context");
    context.with(|ctx| run(&ctx))
}

/// Numbers compare by value: `3` and `3.0` are the same JSON number.
fn normalize(value: Value) -> Value {
    match value {
        Value::Number(number) => Number::from_f64(number.as_f64().expect("finite number"))
            .map_or(Value::Number(number), Value::Number),
        Value::Array(items) => Value::Array(items.into_iter().map(normalize).collect()),
        Value::Object(map) => Value::Object(
            map.into_iter()
                .map(|(key, value)| (key, normalize(value)))
                .collect(),
        ),
        other => other,
    }
}

/// Encoding natively equals `serde_json::to_string` then `JSON.parse`.
pub(crate) fn assert_encodes_like_serde<T: JsEncode + Serialize + Debug>(value: &T) {
    let reference = serde_json::to_value(value).expect("serialize reference");
    let native = with_js(|ctx| {
        let encoded = value.encode_js(ctx).expect("encode natively");
        let text = ctx
            .json_stringify(encoded)
            .expect("stringify")
            .expect("JSON text")
            .to_string()
            .expect("utf-8 JSON text");
        serde_json::from_str::<Value>(&text).expect("parse native JSON")
    });
    assert_eq!(
        normalize(native),
        normalize(reference),
        "encoding {value:?}"
    );
}

/// Decoding `JSON.parse(text)` natively matches `serde_json::from_str(text)`, including
/// whether it fails.
pub(crate) fn assert_decodes_like_serde<T: JsDecode + DeserializeOwned + PartialEq + Debug>(
    text: &str,
) {
    let reference = serde_json::from_str::<T>(text);
    let native = with_js(|ctx| {
        let parsed = ctx.json_parse(text).expect("JSON.parse");
        T::decode_js(ctx, parsed).map_err(|error| error.to_string())
    });
    match (native, reference) {
        (Ok(native), Ok(reference)) => assert_eq!(native, reference, "decoding {text}"),
        (Err(_), Err(_)) => {}
        (native, reference) => {
            panic!("decoding {text}: native {native:?} but serde_json {reference:?}")
        }
    }
}

/// The schema accepts exactly the keys serde writes.
fn assert_schema_accepts<T: TsSchema + Serialize + Debug>(value: &T) {
    let reference = serde_json::to_value(value).expect("serialize reference");
    T::validate_json_strict(&reference)
        .unwrap_or_else(|error| panic!("schema rejects {reference} for {value:?}: {error}"));
}

/// Encoding, decoding and schema acceptance all agree with serde for each of `values`.
pub(crate) fn assert_contract<T>(values: &[T])
where
    T: TsSchema + JsEncode + JsDecode + Serialize + DeserializeOwned + PartialEq + Debug,
{
    for value in values {
        assert_encodes_like_serde(value);
        assert_decodes_like_serde::<T>(&serde_json::to_string(value).expect("serialize"));
        assert_schema_accepts(value);
    }
}

/// The error message of decoding the JavaScript value `source`, which must fail.
pub(crate) fn decode_error<T: JsDecode + Debug>(source: &str) -> String {
    with_js(|ctx| {
        let value = ctx
            .eval::<JsValue<'_>, _>(format!("({source})"))
            .expect("evaluate test value");
        let error = T::decode_js(ctx, value).expect_err("decoding must fail");
        error.to_string()
    })
}

/// The error message of encoding `value`, which must fail.
pub(crate) fn encode_error<T: JsEncode>(value: &T) -> String {
    with_js(|ctx| {
        value
            .encode_js(ctx)
            .expect_err("encoding must fail")
            .to_string()
    })
}
