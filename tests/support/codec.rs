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
    let reference: Value =
        serde_json::from_str(&serde_json::to_string(value).expect("serialize reference"))
            .expect("parse reference");
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
/// whether it fails and, when both fail, where and why (see [`assert_same_failure`]).
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
        (Err(native), Err(reference)) => assert_same_failure(text, &native, &reference),
        (native, reference) => {
            panic!("decoding {text}: native {native:?} but serde_json {reference:?}")
        }
    }
}

/// Two decoders that both refuse `text` must refuse it for the same reason, not for
/// unrelated ones. `serde_json` writes no path, only the position where it gave up, so:
///
/// - a type or value that does not fit (`invalid type`, `invalid value`) must fail at the
///   same path in the native message as the path of the JSON value ending at that position;
/// - the name serde quotes in backticks (the field or variant it refused) must appear in
///   the native message, a missing field as `name: missing field`.
fn assert_same_failure(text: &str, native: &str, reference: &serde_json::Error) {
    let message = reference.to_string();
    let names: Vec<&str> = message.split('`').skip(1).step_by(2).collect();
    // The first quoted name is the one that was refused; the ones after it are the list
    // of what was expected, which serde draws from its own tables.
    if let Some(name) = names.first() {
        assert!(
            native.contains(name),
            "decoding {text}: native `{native}` does not name `{name}` as serde_json `{message}`"
        );
    }
    if message.starts_with("missing field") {
        let name = names.first().expect("serde names the missing field");
        assert!(
            native.contains(&format!("{name}: missing field")),
            "decoding {text}: native `{native}` but serde_json `{message}`"
        );
    }
    if message.starts_with("invalid type") || message.starts_with("invalid value") {
        assert_eq!(
            native_path(native),
            reference_path(text, reference),
            "decoding {text}: native `{native}` fails elsewhere than serde_json `{message}`"
        );
    }
}

/// The segments of the path a native message starts with (`inner[1].id: expected ...`).
fn native_path(message: &str) -> Vec<String> {
    let Some((path, _)) = message.split_once(": ") else {
        return Vec::new();
    };
    if path.contains(' ') {
        return Vec::new();
    }
    path.split(['.', '[', ']'])
        .filter(|segment| !segment.is_empty())
        .map(str::to_owned)
        .collect()
}

/// The path of the JSON value of `text` that ends where `error` was reported: object keys
/// and array indices, outermost first.
fn reference_path(text: &str, error: &serde_json::Error) -> Vec<String> {
    enum Frame {
        Object { key: String, expect_key: bool },
        Array(usize),
    }
    let before: usize = text
        .split_inclusive('\n')
        .take(error.line().saturating_sub(1))
        .map(str::len)
        .sum();
    let mut end = (before + error.column()).min(text.len());
    while !text.is_char_boundary(end) {
        end -= 1;
    }
    let mut frames: Vec<Frame> = Vec::new();
    let mut chars = text[..end].chars();
    while let Some(character) = chars.next() {
        match character {
            '{' => frames.push(Frame::Object {
                key: String::new(),
                expect_key: true,
            }),
            '[' => frames.push(Frame::Array(0)),
            '}' | ']' => {
                frames.pop();
            }
            ',' => match frames.last_mut() {
                Some(Frame::Object { expect_key, .. }) => *expect_key = true,
                Some(Frame::Array(index)) => *index += 1,
                None => {}
            },
            '"' => {
                let mut string = String::new();
                while let Some(next) = chars.next() {
                    match next {
                        '\\' => {
                            string.push(next);
                            string.extend(chars.next());
                        }
                        '"' => break,
                        other => string.push(other),
                    }
                }
                if let Some(Frame::Object { key, expect_key }) = frames.last_mut()
                    && *expect_key
                {
                    *key = string;
                    *expect_key = false;
                }
            }
            _ => {}
        }
    }
    frames
        .into_iter()
        .map(|frame| match frame {
            Frame::Object { key, .. } => key,
            Frame::Array(index) => index.to_string(),
        })
        .collect()
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
