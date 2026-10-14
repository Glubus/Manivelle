//! Native codecs for `uuid`, `chrono` and `glam` types follow each crate's `serde`
//! impl through `serde_json`: encoding then `JSON.stringify` equals
//! `serde_json::to_string`, and `JSON.parse` of serde's text then decoding gives the
//! value back.

#![cfg(all(
    feature = "derive",
    feature = "uuid",
    feature = "chrono",
    feature = "glam"
))]

use std::fmt::Debug;

use chrono::{DateTime, FixedOffset, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use glam::{
    DVec2, DVec3, DVec4, IVec2, IVec3, IVec4, Mat2, Mat3, Mat4, Quat, UVec2, UVec3, UVec4, Vec2,
    Vec3, Vec4,
};
use manivelle::js::{Context, Ctx, Function, Runtime, Value as JsValue};
use manivelle::{JsDecode, JsEncode, TsSchema, TsType};
use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use serde_json::{Number, Value};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

fn with_js<R>(run: impl FnOnce(&Ctx<'_>) -> R) -> R {
    let runtime = Runtime::new().expect("create runtime");
    let context = Context::full(&runtime).expect("create context");
    context.with(|ctx| run(&ctx))
}

/// Numbers compare by value: serde writes `1.0` where `JSON.stringify` writes `1`.
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

/// `JSON.stringify` of the native encoding.
fn native_json<T: JsEncode>(value: &T) -> String {
    with_js(|ctx| {
        let encoded = value.encode_js(ctx).expect("encode natively");
        ctx.json_stringify(encoded)
            .expect("stringify")
            .expect("JSON text")
            .to_string()
            .expect("utf-8 JSON text")
    })
}

fn assert_encodes_like_serde<T: JsEncode + Serialize + Debug>(value: &T) {
    let reference = serde_json::to_string(value).expect("serialize reference");
    let native = native_json(value);
    let parse = |text: &str| normalize(serde_json::from_str::<Value>(text).expect("parse JSON"));
    assert_eq!(parse(&native), parse(&reference), "encoding {value:?}");
}

/// Decoding `JSON.parse(text)` natively matches `serde_json::from_str(text)`, including
/// whether it fails.
fn assert_decodes_like_serde<T: JsDecode + DeserializeOwned + PartialEq + Debug>(text: &str) {
    let reference = serde_json::from_str::<T>(text);
    let native = decode_json::<T>(text);
    match (native, reference) {
        (Ok(native), Ok(reference)) => assert_eq!(native, reference, "decoding {text}"),
        (Err(_), Err(_)) => {}
        (native, reference) => {
            panic!("decoding {text}: native {native:?} but serde_json {reference:?}")
        }
    }
}

fn decode_json<T: JsDecode>(text: &str) -> Result<T, String> {
    with_js(|ctx| {
        let parsed = ctx.json_parse(text).expect("JSON.parse");
        T::decode_js(ctx, parsed).map_err(|error| error.to_string())
    })
}

/// Encodes like serde, decodes serde's text back to the same value, and the schema
/// accepts serde's JSON.
fn assert_contract<T>(values: &[T])
where
    T: TsSchema + JsEncode + JsDecode + Serialize + DeserializeOwned + PartialEq + Debug,
{
    for value in values {
        assert_encodes_like_serde(value);
        let text = serde_json::to_string(value).expect("serialize reference");
        assert_eq!(
            decode_json::<T>(&text).as_ref(),
            Ok(value),
            "decoding {text}"
        );
        let reference = serde_json::to_value(value).expect("serialize reference");
        T::validate_json_strict(&reference)
            .unwrap_or_else(|error| panic!("schema rejects {reference}: {error}"));
    }
}

fn decode_error<T: JsDecode + Debug>(source: &str) -> String {
    with_js(|ctx| {
        let value = ctx
            .eval::<JsValue<'_>, _>(format!("({source})"))
            .expect("evaluate test value");
        T::decode_js(ctx, value)
            .expect_err("decoding must fail")
            .to_string()
    })
}

/// Evaluates `check`, a JS function source, against the encoded value.
fn check_encoded<T: JsEncode>(value: &T, check: &str) -> bool {
    with_js(|ctx| {
        let encoded = value.encode_js(ctx).expect("encode natively");
        let check: Function<'_> = ctx.eval(check).expect("compile check");
        check.call((encoded,)).expect("run check")
    })
}

fn numbers(len: usize) -> TsType {
    TsType::Tuple(vec![TsType::Number; len])
}

// ---------------------------------------------------------------------------
// uuid
// ---------------------------------------------------------------------------

#[test]
fn uuids_cross_as_hyphenated_lowercase_strings() {
    let sample = Uuid::from_u128(0xF916_8C5E_CEB2_4FAA_B6BF_329B_F39F_A1E4);
    assert_contract(&[Uuid::nil(), Uuid::max(), sample]);
    assert_eq!(
        native_json(&sample),
        r#""f9168c5e-ceb2-4faa-b6bf-329bf39fa1e4""#
    );
    assert_eq!(Uuid::ts_type(), TsType::String);
}

#[test]
fn uuids_decode_every_form_serde_accepts() {
    for text in [
        r#""f9168c5eceb24faab6bf329bf39fa1e4""#,
        r#""F9168C5E-CEB2-4FAA-B6BF-329BF39FA1E4""#,
        r#""{f9168c5e-ceb2-4faa-b6bf-329bf39fa1e4}""#,
        r#""urn:uuid:f9168c5e-ceb2-4faa-b6bf-329bf39fa1e4""#,
        r#""f9168c5e-ceb2-4faa-b6bf-329bf39fa1e""#,
        r#""not a uuid""#,
        r#""""#,
        "[249,22,140,94,206,178,79,170,182,191,50,155,243,159,161,228]",
        "42",
    ] {
        assert_decodes_like_serde::<Uuid>(text);
    }
}

// ---------------------------------------------------------------------------
// chrono
// ---------------------------------------------------------------------------

fn date(year: i32, month: u32, day: u32) -> NaiveDate {
    NaiveDate::from_ymd_opt(year, month, day).expect("valid date")
}

fn time(hour: u32, minute: u32, second: u32, nano: u32) -> NaiveTime {
    NaiveTime::from_hms_nano_opt(hour, minute, second, nano).expect("valid time")
}

fn offset(seconds: i32) -> FixedOffset {
    FixedOffset::east_opt(seconds).expect("valid offset")
}

fn naive_date_times() -> Vec<NaiveDateTime> {
    vec![
        date(1970, 1, 1).and_time(time(0, 0, 0, 0)),
        // Leap day, with nanoseconds.
        date(2024, 2, 29).and_time(time(23, 59, 59, 123_456_789)),
        // Leap second, milliseconds and microseconds.
        date(2016, 12, 31).and_time(time(23, 59, 59, 1_500_000_000)),
        date(2000, 2, 29).and_time(time(12, 30, 0, 250_000)),
        // Years outside 0..=9999 carry an explicit sign.
        date(-1, 3, 1).and_time(time(1, 2, 3, 0)),
        date(12_345, 6, 7).and_time(time(8, 9, 10, 0)),
    ]
}

#[test]
fn naive_dates_and_times_follow_serde() {
    let date_times = naive_date_times();
    assert_contract(&date_times);
    assert_contract(&date_times.iter().map(|at| at.date()).collect::<Vec<_>>());
    assert_contract(&date_times.iter().map(|at| at.time()).collect::<Vec<_>>());
    assert_contract(&[NaiveDate::MIN, NaiveDate::MAX]);
    assert_eq!(
        native_json(&date(2024, 2, 29)),
        r#""2024-02-29""#,
        "leap day"
    );
    for ts_type in [
        NaiveDate::ts_type(),
        NaiveTime::ts_type(),
        NaiveDateTime::ts_type(),
    ] {
        assert_eq!(ts_type, TsType::String);
    }
}

#[test]
fn date_times_follow_serde_rfc3339() {
    let utc: Vec<DateTime<Utc>> = naive_date_times()
        .into_iter()
        .map(|at| at.and_utc())
        .collect();
    assert_contract(&utc);
    assert_eq!(native_json(&utc[1]), r#""2024-02-29T23:59:59.123456789Z""#);

    let offsets = [0, -5 * 3600 - 30 * 60, 14 * 3600, -12 * 3600, 3600];
    let fixed: Vec<DateTime<FixedOffset>> = naive_date_times()
        .into_iter()
        .zip(offsets.into_iter().cycle())
        .map(|(at, seconds)| {
            offset(seconds)
                .from_local_datetime(&at)
                .single()
                .expect("unambiguous local time")
        })
        .collect();
    assert_contract(&fixed);
    assert_eq!(
        native_json(&fixed[1]),
        r#""2024-02-29T23:59:59.123456789-05:30""#
    );

    assert_eq!(DateTime::<Utc>::ts_type(), TsType::String);
    assert_eq!(DateTime::<FixedOffset>::ts_type(), TsType::String);
}

#[test]
fn offsets_with_seconds_round_to_minutes_like_serde() {
    let at = date(2024, 2, 29).and_time(time(12, 0, 0, 0));
    for seconds in [3600 + 150, -(3600 + 29), -10, 45 * 60 + 31] {
        let value = offset(seconds)
            .from_local_datetime(&at)
            .single()
            .expect("unambiguous local time");
        assert_encodes_like_serde(&value);
    }
}

#[test]
fn date_times_decode_what_serde_accepts() {
    for text in [
        r#""2024-02-29T23:59:59+02:00""#,
        r#""2024-02-29T23:59:59.5-00:30""#,
        r#""2024-02-29 23:59:59Z""#,
        r#""2024-02-29t23:59:59z""#,
        r#""+12345-06-07T08:09:10Z""#,
        r#""2023-02-29T00:00:00Z""#,
        r#""2024-02-29T24:00:00Z""#,
        r#""2024-02-29T12:00:00""#,
        r#""yesterday""#,
        "1709251199",
    ] {
        assert_decodes_like_serde::<DateTime<Utc>>(text);
        assert_decodes_like_serde::<DateTime<FixedOffset>>(text);
        assert_decodes_like_serde::<NaiveDateTime>(text);
    }
    for text in [r#""2024-02-29""#, r#""2023-02-29""#, r#""+12345-06-07""#] {
        assert_decodes_like_serde::<NaiveDate>(text);
    }
    for text in [
        r#""23:59:60.5""#,
        r#""7:05""#,
        r#""12:00""#,
        r#""25:00:00""#,
    ] {
        assert_decodes_like_serde::<NaiveTime>(text);
    }
}

// ---------------------------------------------------------------------------
// glam
// ---------------------------------------------------------------------------

#[test]
fn float_vectors_and_quaternions_follow_serde() {
    assert_contract(&[Vec2::ZERO, Vec2::new(1.1, -0.5), Vec2::MAX]);
    assert_contract(&[Vec3::new(0.1, 2.5, -3.75), Vec3::MIN]);
    assert_contract(&[Vec4::new(1.0, 2.0, 3.0, 4.0), Vec4::splat(1e-7)]);
    assert_contract(&[DVec2::new(0.1, 1e300)]);
    assert_contract(&[DVec3::new(-0.0, f64::MIN, 5.5)]);
    assert_contract(&[DVec4::new(0.1, 0.2, 0.3, 0.4)]);
    assert_contract(&[Quat::IDENTITY, Quat::from_xyzw(0.1, 0.2, 0.3, 0.9)]);
    assert_eq!(native_json(&Vec2::new(1.1, -0.5)), "[1.1,-0.5]");
    for (ts_type, len) in [
        (Vec2::ts_type(), 2),
        (Vec3::ts_type(), 3),
        (Vec4::ts_type(), 4),
        (DVec2::ts_type(), 2),
        (DVec3::ts_type(), 3),
        (DVec4::ts_type(), 4),
        (Quat::ts_type(), 4),
    ] {
        assert_eq!(ts_type, numbers(len));
    }
}

#[test]
fn non_finite_vector_components_follow_the_float_rules() {
    let value = Vec3::new(f32::NAN, f32::INFINITY, 1.0);
    // `JSON.stringify` turns them into `null`, exactly like serde_json's text.
    assert_encodes_like_serde(&value);
    assert!(check_encoded(
        &value,
        "v => Number.isNaN(v[0]) && v[1] === Infinity && v[2] === 1"
    ));
    let decoded = with_js(|ctx| {
        let source = ctx
            .eval::<JsValue<'_>, _>("[NaN, -Infinity, 2]")
            .expect("evaluate");
        Vec3::decode_js(ctx, source).expect("decode non-finite components")
    });
    assert!(decoded.x.is_nan());
    assert_eq!((decoded.y, decoded.z), (f32::NEG_INFINITY, 2.0));
}

#[test]
fn integer_vectors_follow_serde_within_their_range() {
    assert_contract(&[IVec2::new(i32::MIN, i32::MAX), IVec2::ZERO]);
    assert_contract(&[IVec3::new(-1, 0, 1)]);
    assert_contract(&[IVec4::new(i32::MIN, -7, 7, i32::MAX)]);
    assert_contract(&[UVec2::new(0, u32::MAX)]);
    assert_contract(&[UVec3::new(1, 2, 3)]);
    assert_contract(&[UVec4::new(0, 1, u32::MAX - 1, u32::MAX)]);
    for text in ["[2147483648,0]", "[1.5,0]", "[-1,0]"] {
        assert_decodes_like_serde::<IVec2>(text);
        assert_decodes_like_serde::<UVec2>(text);
    }
    assert_eq!(IVec3::ts_type(), numbers(3));
    assert_eq!(UVec4::ts_type(), numbers(4));
}

#[test]
fn matrices_cross_column_major_like_serde() {
    let mat2 = Mat2::from_cols_array(&[1.0, 2.0, 3.0, 4.0]);
    let mat3 = Mat3::from_cols_array(&[1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.5]);
    let mat4 = Mat4::from_cols_array(&std::array::from_fn(|index| index as f32 * 0.1));
    assert_contract(&[mat2, Mat2::IDENTITY]);
    assert_contract(&[mat3, Mat3::IDENTITY]);
    assert_contract(&[mat4, Mat4::IDENTITY]);
    assert_eq!(native_json(&mat2), "[1,2,3,4]");
    assert_eq!(Mat2::ts_type(), numbers(4));
    assert_eq!(Mat3::ts_type(), numbers(9));
    assert_eq!(Mat4::ts_type(), numbers(16));
}

#[test]
fn glam_values_need_exactly_their_component_count() {
    for text in [
        "[1,2]",
        "[1,2,3,4]",
        "[]",
        "{\"x\":1,\"y\":2,\"z\":3}",
        "[1,\"2\",3]",
    ] {
        assert_decodes_like_serde::<Vec3>(text);
    }
    for text in ["[1,2,3]", "[1,2,3,4,5]"] {
        assert_decodes_like_serde::<Quat>(text);
        assert_decodes_like_serde::<Mat2>(text);
    }
}

// ---------------------------------------------------------------------------
// Inside derived structs
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct Entity {
    id: Uuid,
    spawned_at: DateTime<Utc>,
    birthday: NaiveDate,
    position: Vec3,
    cell: IVec2,
    orientation: Quat,
    transform: Mat4,
}

fn entity() -> Entity {
    Entity {
        id: Uuid::from_u128(0x0123_4567_89AB_CDEF_0123_4567_89AB_CDEF),
        spawned_at: date(2024, 2, 29)
            .and_time(time(6, 30, 0, 5_000_000))
            .and_utc(),
        birthday: date(2000, 2, 29),
        position: Vec3::new(1.5, -2.25, 0.1),
        cell: IVec2::new(-3, 4),
        orientation: Quat::from_xyzw(0.0, 0.0, 0.0, 1.0),
        transform: Mat4::from_translation(Vec3::new(1.0, 2.0, 3.0)),
    }
}

#[test]
fn derived_structs_carry_ecosystem_fields_like_serde() {
    assert_contract(&[entity()]);
}

#[test]
fn decode_errors_name_the_ecosystem_field_path() {
    let valid = serde_json::to_value(entity()).expect("serialize entity");
    let with = |field: &str, source: &str| {
        let mut object = valid.clone();
        object[field] = Value::Null;
        let text = object.to_string();
        let source = text.replace(
            &format!("\"{field}\":null"),
            &format!("\"{field}\":{source}"),
        );
        decode_error::<Entity>(&source)
    };

    let id = with("id", r#""not-a-uuid""#);
    assert!(
        id.contains(r#"id: expected a UUID, got "not-a-uuid""#),
        "{id}"
    );
    let id_type = with("id", "42");
    assert!(id_type.contains("id: expected string, got 42"), "{id_type}");

    let spawned = with("spawnedAt", r#""2023-02-29T00:00:00Z""#);
    assert!(
        spawned.contains(
            r#"spawnedAt: expected an RFC 3339 date and time, got "2023-02-29T00:00:00Z""#
        ),
        "{spawned}"
    );
    let birthday = with("birthday", r#""2000-13-01""#);
    assert!(
        birthday.contains(r#"birthday: expected an ISO 8601 date, got "2000-13-01""#),
        "{birthday}"
    );

    let short = with("position", "[1, 2]");
    assert!(
        short.contains("position: expected array of length 3, got length 2"),
        "{short}"
    );
    let component = with("position", r#"[1, "2", 3]"#);
    assert!(
        component.contains("position[1]: expected number"),
        "{component}"
    );
    let range = with("cell", "[0, 2147483648]");
    assert!(range.contains("cell[1]: "), "{range}");
    let matrix = with("transform", "[1, 2, 3, 4]");
    assert!(
        matrix.contains("transform: expected array of length 16, got length 4"),
        "{matrix}"
    );
}
