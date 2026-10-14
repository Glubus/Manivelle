//! Native codecs for standard types follow the `serde_json` reference:
//! Rust -> JS equals `serde_json::to_string` then `JSON.parse`, and JS -> Rust equals
//! `JSON.stringify` then `serde_json::from_str`, apart from the documented stricter rules.

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque};
use std::fmt::Debug;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::PathBuf;

use manivelle::js::{Context, Ctx, Runtime, Value as JsValue};
use manivelle::{JsDecode, JsEncode, NativeBytes};
use serde::Serialize;
use serde_json::{Number, Value, json};

const MAX_SAFE: i64 = (1 << 53) - 1;

fn with_js<R>(run: impl FnOnce(&Ctx<'_>) -> R) -> R {
    let runtime = Runtime::new().expect("create runtime");
    let context = Context::full(&runtime).expect("create context");
    context.with(|ctx| run(&ctx))
}

fn eval<'js>(ctx: &Ctx<'js>, source: &str) -> JsValue<'js> {
    ctx.eval::<JsValue<'js>, _>(format!("({source})"))
        .expect("evaluate test value")
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

fn assert_encodes_like_serde<T: JsEncode + Serialize + Debug>(value: &T) {
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

fn assert_decodes_like_serde<T: JsDecode + Serialize + PartialEq + Debug>(value: &T) {
    let text = serde_json::to_string(value).expect("serialize reference");
    let decoded = with_js(|ctx| {
        let parsed = ctx.json_parse(text.clone()).expect("JSON.parse");
        T::decode_js(ctx, parsed).expect("decode natively")
    });
    assert_eq!(&decoded, value, "decoding {text}");
}

fn assert_round_trips<T>(values: &[T])
where
    T: JsEncode + JsDecode + Serialize + PartialEq + Debug,
{
    for value in values {
        assert_encodes_like_serde(value);
        assert_decodes_like_serde(value);
    }
}

fn decode<T: JsDecode>(source: &str) -> Result<T, String> {
    with_js(|ctx| T::decode_js(ctx, eval(ctx, source)).map_err(|error| error.to_string()))
}

fn decode_error<T: JsDecode + Debug>(source: &str) -> String {
    match decode::<T>(source) {
        Ok(value) => panic!("{source} decoded as {value:?}"),
        Err(error) => error,
    }
}

fn encode_error<T: JsEncode>(value: &T) -> String {
    with_js(|ctx| match value.encode_js(ctx) {
        Ok(_) => panic!("value encoded"),
        Err(error) => error.to_string(),
    })
}

/// Evaluates `check`, a JS function source, against the encoded value.
fn check_encoded<T: JsEncode>(value: &T, check: &str) -> bool {
    with_js(|ctx| {
        let encoded = value.encode_js(ctx).expect("encode natively");
        let check: manivelle::js::Function<'_> = ctx.eval(check).expect("compile check");
        check.call((encoded,)).expect("run check")
    })
}

#[test]
fn booleans_and_unit_follow_serde() {
    assert_round_trips(&[true, false]);
    assert_round_trips(&[()]);
    assert_eq!(decode::<()>("undefined"), Ok(()));
    assert!(decode::<bool>("1").is_err());
}

#[test]
fn integers_follow_serde_within_their_range() {
    assert_round_trips(&[i8::MIN, -1, 0, i8::MAX]);
    assert_round_trips(&[i16::MIN, i16::MAX]);
    assert_round_trips(&[i32::MIN, i32::MAX]);
    assert_round_trips(&[0_u8, u8::MAX]);
    assert_round_trips(&[u16::MAX]);
    assert_round_trips(&[0_u32, u32::MAX]);
    assert_round_trips(&[-MAX_SAFE, -1, 0, i64::from(i32::MAX) + 1, MAX_SAFE]);
    assert_round_trips(&[0_u64, u64::from(u32::MAX) + 1, MAX_SAFE as u64]);
    assert_round_trips(&[-(MAX_SAFE as i128), MAX_SAFE as i128]);
    assert_round_trips(&[MAX_SAFE as u128]);
    assert_round_trips(&[isize::MIN.max(-(MAX_SAFE as isize)), 7]);
    assert_round_trips(&[usize::MIN, 1 << 40]);
}

#[test]
fn integers_outside_the_safe_range_fail_both_ways() {
    let beyond = MAX_SAFE + 1;

    assert!(encode_error(&(beyond as u64)).contains("safe integer range"));
    assert!(encode_error(&-beyond).contains("safe integer range"));
    assert!(encode_error(&u128::MAX).contains("safe integer range"));
    assert!(decode_error::<u64>("2 ** 53").contains("safe integer in u64 range"));
    assert!(decode_error::<i64>("-(2 ** 53)").contains("safe integer in i64 range"));
    assert_eq!(decode::<u64>("2 ** 53 - 1"), Ok(MAX_SAFE as u64));
}

#[test]
fn integers_reject_out_of_range_fractional_and_non_numeric_values() {
    assert_eq!(
        decode_error::<u8>("300"),
        "Error converting from js 'int' into type 'u8': expected integer in u8 range, got 300"
    );
    assert!(decode_error::<u8>("-1").contains("got -1"));
    assert!(decode_error::<i32>("1.5").contains("expected integer in i32 range, got 1.5"));
    assert!(decode_error::<u32>("NaN").contains("got NaN"));
    assert!(decode_error::<u32>("'7'").contains("got string"));
    assert_eq!(decode::<u32>("3000000000"), Ok(3_000_000_000));
    assert_eq!(decode::<i16>("-0"), Ok(0));
}

#[test]
fn floats_follow_serde_for_finite_values() {
    assert_round_trips(&[0.0_f64, -0.0, 1.5, -2.25, 0.1, 1e300, 5e-324, f64::MAX]);
    assert_round_trips(&[0.0_f32, 1.1, -3.5, 1e-45, 3e10, f32::MAX, f32::MIN_POSITIVE]);
}

#[test]
fn non_finite_floats_cross_as_nan_and_infinities() {
    assert!(check_encoded(&f64::NAN, "(v) => Number.isNaN(v)"));
    assert!(check_encoded(&f64::INFINITY, "(v) => v === Infinity"));
    assert!(check_encoded(&f32::NEG_INFINITY, "(v) => v === -Infinity"));
    assert!(decode::<f64>("NaN").expect("decode NaN").is_nan());
    assert_eq!(decode::<f64>("Infinity"), Ok(f64::INFINITY));
    assert_eq!(decode::<f32>("-Infinity"), Ok(f32::NEG_INFINITY));
}

#[test]
fn negative_zero_keeps_its_sign() {
    assert!(check_encoded(&-0.0_f64, "(v) => Object.is(v, -0)"));
    assert!(
        decode::<f64>("-0")
            .expect("decode negative zero")
            .is_sign_negative()
    );
}

#[test]
fn text_types_follow_serde() {
    assert_round_trips(&[
        String::new(),
        "héllo".to_owned(),
        "line\nbreak \"quoted\"".to_owned(),
        "😀".to_owned(),
    ]);
    assert_round_trips(&['a', 'é', '😀']);
    assert_round_trips(&[PathBuf::from("dir/file.txt")]);
    assert_round_trips(&[
        IpAddr::from([127, 0, 0, 1]),
        IpAddr::V6(Ipv6Addr::LOCALHOST),
    ]);
    assert_round_trips(&[Ipv4Addr::new(10, 0, 0, 255)]);
    assert_round_trips(&["fe80::1:2".parse::<Ipv6Addr>().expect("ipv6")]);
}

#[test]
fn text_types_reject_other_shapes() {
    assert!(decode_error::<char>("'ab'").contains("exactly one character, got 2 characters"));
    assert!(decode_error::<char>("''").contains("got 0 characters"));
    assert!(decode_error::<String>("12").contains("expected string, got 12"));
    assert!(decode_error::<String>("'\\ud800'").contains("lone surrogates"));
    assert!(decode_error::<Ipv4Addr>("'::1'").contains("expected an IPv4 address, got \"::1\""));
    assert!(decode_error::<IpAddr>("'not an ip'").contains("expected an IP address"));
}

#[test]
fn options_map_null_and_undefined_to_none() {
    assert_round_trips(&[Some(3_u8), None]);
    assert_round_trips(&[Some(Some(1_u8)), None]);
    assert_eq!(decode::<Option<u8>>("undefined"), Ok(None));
    assert_eq!(decode::<Option<u8>>("null"), Ok(None));
}

#[test]
fn sequences_follow_serde() {
    assert_round_trips(&[vec![1_u16, 2, 3], Vec::new()]);
    assert_round_trips(&[VecDeque::from(["a".to_owned(), "b".to_owned()])]);
    assert_round_trips(&[[1_u8, 2, 3]]);
    assert_round_trips(&[vec![true, false].into_boxed_slice()]);
    assert_round_trips(&[Box::new(9_u8)]);
    assert_round_trips(&[HashSet::from([1_u32, 5, 9])]);
    assert_round_trips(&[BTreeSet::from(["x".to_owned(), "y".to_owned()])]);
    assert_round_trips(&[vec![vec![Some(1_i8)], vec![None]]]);
}

#[test]
fn sequences_require_arrays_and_exact_fixed_lengths() {
    assert!(decode_error::<[u8; 3]>("[1, 2]").contains("expected array of length 3, got length 2"));
    assert!(decode_error::<Vec<u8>>("{ length: 1, 0: 1 }").contains("expected array, got object"));
    assert_eq!(
        decode::<HashSet<u8>>("[1, 1, 2]"),
        Ok(HashSet::from([1, 2]))
    );
}

#[test]
fn huge_sparse_arrays_fail_at_their_first_hole_without_reserving_their_length() {
    let sparse = "(() => { const items = []; items.length = 2 ** 32 - 1; return items; })()";

    let error = decode_error::<Vec<u64>>(sparse);

    assert!(
        error.contains("[0]: expected safe integer in u64 range, got undefined"),
        "{error}"
    );
    assert!(
        decode_error::<[u8; 3]>(sparse)
            .contains("expected array of length 3, got length 4294967295")
    );
}

#[test]
fn tuples_follow_serde_and_require_exact_length() {
    assert_round_trips(&[(7_u8,)]);
    assert_round_trips(&[(1_u8, "two".to_owned())]);
    assert_round_trips(&[(
        true,
        2_u8,
        'c',
        "d".to_owned(),
        5.5_f64,
        -6_i64,
        Some(7_u8),
        vec![8_u8],
        (),
        10_u16,
        -11_i8,
        12_u32,
    )]);
    assert!(
        decode_error::<(u8, u8)>("[1, 2, 3]").contains("expected array of length 2, got length 3")
    );
    assert!(decode_error::<(u8, u8)>("[1]").contains("got length 1"));
}

#[test]
fn string_keyed_maps_follow_serde() {
    assert_round_trips(&[HashMap::from([
        ("a".to_owned(), 1_u8),
        ("b c".to_owned(), 2),
    ])]);
    assert_round_trips(&[BTreeMap::from([("list".to_owned(), vec![1_u8, 2])])]);
    assert!(decode_error::<HashMap<String, u8>>("[1]").contains("expected object, got array"));
}

#[test]
fn maps_skip_properties_json_drops() {
    let decoded = decode::<BTreeMap<String, Option<u8>>>("{ a: 1, b: undefined, c: () => 1 }");

    assert_eq!(decoded, Ok(BTreeMap::from([("a".to_owned(), Some(1))])));
}

#[test]
fn integer_keyed_maps_round_trip_with_decimal_keys() {
    assert_round_trips(&[HashMap::from([
        (1_u32, "a".to_owned()),
        (20, "b".to_owned()),
    ])]);
    assert_round_trips(&[BTreeMap::from([(-5_i64, true), (0, false), (7, true)])]);
    assert_round_trips(&[BTreeMap::from([(1_u64 << 60, 1_u8), (u64::MAX, 2)])]);
    assert_eq!(
        decode::<BTreeMap<u8, bool>>("{ 1: true, 2: false }"),
        Ok(BTreeMap::from([(1, true), (2, false)]))
    );
}

#[test]
fn integer_keyed_maps_reject_keys_that_are_not_canonical_integers() {
    assert!(
        decode_error::<HashMap<u32, bool>>("{ abc: true }")
            .contains("[abc]: expected decimal u32 key, got \"abc\"")
    );
    assert!(decode_error::<HashMap<u32, bool>>("{ '01': true }").contains("got \"01\""));
    assert!(decode_error::<HashMap<u8, bool>>("{ 256: true }").contains("got \"256\""));
    assert!(decode_error::<BTreeMap<i8, bool>>("{ '+1': true }").contains("got \"+1\""));
}

#[test]
fn json_values_cross_structurally() {
    assert_round_trips(&[json!({
        "list": [1, 2.5, null, true, "text"],
        "nested": { "negative": -3, "empty": {} },
    })]);
    let large = decode::<Value>("3000000000").expect("decode large integer");
    assert_eq!(large.as_u64(), Some(3_000_000_000));
    assert_eq!(decode::<Value>("2.5"), Ok(json!(2.5)));
    assert!(encode_error(&json!(u64::MAX)).contains("safe integer range"));
}

#[test]
fn proto_keys_of_maps_and_json_objects_stay_own_properties() {
    let own_proto = "(o) => Object.getPrototypeOf(o) === Object.prototype \
        && Object.prototype.hasOwnProperty.call(o, '__proto__') \
        && o.__proto__ !== Object.prototype && o.polluted === undefined";
    let map = HashMap::from([(String::from("__proto__"), json!({ "polluted": true }))]);
    let tree = BTreeMap::from([(String::from("__proto__"), 1_u8)]);
    let value = json!({ "__proto__": { "polluted": true } });

    assert!(check_encoded(&map, own_proto));
    assert!(check_encoded(&tree, own_proto));
    assert!(check_encoded(&value, own_proto));
    assert_round_trips(&[map]);
    assert_round_trips(&[value]);
}

#[test]
fn native_bytes_encode_as_uint8_array() {
    let bytes = NativeBytes::new(vec![1, 2, 255]);

    assert!(check_encoded(
        &bytes,
        "(v) => v instanceof Uint8Array && v.length === 3 && v[2] === 255"
    ));
}

#[test]
fn native_bytes_decode_from_binary_views_and_byte_arrays() {
    let expected = NativeBytes::new(vec![1, 2, 255]);

    assert_eq!(decode("new Uint8Array([1, 2, 255])"), Ok(expected.clone()));
    assert_eq!(
        decode("new Uint8Array([1, 2, 255]).buffer"),
        Ok(expected.clone())
    );
    assert_eq!(
        decode("new Uint8Array([9, 1, 2, 255]).subarray(1)"),
        Ok(expected.clone())
    );
    assert_eq!(decode("[1, 2, 255]"), Ok(expected.clone()));
    assert_decodes_like_serde(&expected);
    assert!(decode_error::<NativeBytes>("[1, 256]").contains("[1]: expected integer in u8 range"));
    assert!(decode_error::<NativeBytes>("new Int8Array(2)").contains("Uint8Array, ArrayBuffer"));
}

#[test]
fn nested_errors_name_the_failing_path() {
    let error =
        decode_error::<Vec<HashMap<String, (u8, bool)>>>("[{ a: [1, true] }, { b: [300, false] }]");

    assert!(
        error.contains("[1][b][0]: expected integer in u8 range, got 300"),
        "{error}"
    );
}

#[test]
fn encode_errors_name_the_failing_path() {
    let error = encode_error(&vec![(0_u64, 1_u64 << 60)]);

    assert!(error.contains("[0][1]: 1152921504606846976"), "{error}");
}
