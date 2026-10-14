//! Calling exports with number and boolean arguments and results: what crosses, exactly,
//! and what fails with which error.

use manivelle::{Engine, EngineError, EngineOptions};

fn engine() -> Engine {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .load_script(
            "script",
            r#"
            export const echo = (value: unknown): unknown => value;
            export const describe = (value: number): string =>
                Object.is(value, -0) ? "-0" : Number.isNaN(value) ? "nan" : String(value);
            export const kind = (value: unknown): string => typeof value;
            export const sum = (a: number, b: number): number => a + b;
            export const sum9 = (...n: number[]): number => n.reduce((a, b) => a + b, 0);
            export const not = (flag: boolean): boolean => !flag;
            export const mixed = (n: number, text: string, flag: boolean): string => `${n}${text}${flag}`;
            export const half = (): number => 0.5;
            export const big = (): number => 2 ** 40;
            export const unsafe_big = (): number => 2 ** 60;
            export const fraction = (): number => 1.5;
            export const wide = (): number => 300;
            export const negative = (): number => -1;
            export const one = (): number => 1;
            export const text = (): string => "no";
            export const nothing = (): void => {};
            export const boom = (n: number): number => { throw new Error(`boom ${n}`); };
            export const later = async (n: number): Promise<number> => n + 1;
            export const never = (): Promise<number> => new Promise(() => {});
            "#,
        )
        .expect("load script");
    engine
}

fn describe(engine: &Engine, value: f64) -> String {
    engine
        .call("script", "describe", (value,))
        .expect("describe")
}

#[test]
fn floats_cross_exactly_including_negative_zero_nan_and_infinities() {
    let engine = engine();

    assert_eq!(describe(&engine, -0.0), "-0");
    assert_eq!(describe(&engine, 0.0), "0");
    assert_eq!(describe(&engine, f64::NAN), "nan");
    assert_eq!(describe(&engine, f64::INFINITY), "Infinity");
    assert_eq!(describe(&engine, f64::NEG_INFINITY), "-Infinity");
    assert_eq!(describe(&engine, 0.1), "0.1");
    assert_eq!(describe(&engine, 2_147_483_648.0), "2147483648");
    assert_eq!(describe(&engine, -2_147_483_649.0), "-2147483649");
    assert_eq!(describe(&engine, 1e300), "1e+300");
}

#[test]
fn integers_cross_across_the_int32_boundary_and_up_to_the_safe_limit() {
    let engine = engine();
    let echo = |value: serde_json::Value| -> serde_json::Value {
        engine.call("script", "echo", vec![value]).expect("echo")
    };

    let u32_max: f64 = engine.call("script", "sum", (u32::MAX, 0u32)).expect("u32");
    let i32_min: f64 = engine.call("script", "sum", (i32::MIN, 0i8)).expect("i32");
    let safe: f64 = engine
        .call("script", "sum", ((1i64 << 53) - 1, 0u8))
        .expect("safe");
    let small: f64 = engine.call("script", "sum", (-5i16, 2u16)).expect("small");

    assert_eq!(u32_max, 4_294_967_295.0);
    assert_eq!(i32_min, -2_147_483_648.0);
    assert_eq!(safe, 9_007_199_254_740_991.0);
    assert_eq!(small, -3.0);
    assert_eq!(echo(serde_json::json!(7)), serde_json::json!(7));
}

#[test]
fn an_integer_outside_the_safe_range_fails_naming_the_argument() {
    let engine = engine();

    let result = engine.call::<f64>("script", "sum", (1.0, 1i64 << 60));

    let message = result.expect_err("outside the safe range").to_string();
    assert!(message.contains("arguments[1]"), "{message}");
    assert!(message.contains("safe integer"), "{message}");
}

#[test]
fn booleans_cross_both_ways() {
    let engine = engine();

    let a: bool = engine.call("script", "not", (true,)).expect("not true");
    let b: bool = engine.call("script", "not", (false,)).expect("not false");
    let kind: String = engine.call("script", "kind", (true,)).expect("kind");

    assert!(!a && b);
    assert_eq!(kind, "boolean");
}

#[test]
fn a_mix_of_scalars_and_text_and_nine_numbers_still_cross() {
    let engine = engine();

    let mixed: String = engine
        .call("script", "mixed", (1.5, "x", true))
        .expect("mixed");
    let nine: f64 = engine
        .call(
            "script",
            "sum9",
            (1.0, 2.0, 3.0, 4.0, 5.0, 6.0, 7.0, 8.0, 9.0),
        )
        .expect("nine numbers");
    let from_vec: f64 = engine
        .call("script", "sum9", vec![1.0, 2.0, 3.0])
        .expect("vec");
    let none: f64 = engine.call("script", "sum9", ()).expect("no arguments");

    assert_eq!(mixed, "1.5xtrue");
    assert_eq!(nine, 45.0);
    assert_eq!(from_vec, 6.0);
    assert_eq!(none, 0.0);
}

#[test]
fn results_decode_as_the_numbers_they_are() {
    let engine = engine();

    let half: f64 = engine.call("script", "half", ()).expect("half");
    let narrow: f32 = engine.call("script", "half", ()).expect("f32");
    let big: u64 = engine.call("script", "big", ()).expect("2^40");
    let big_signed: i64 = engine.call("script", "big", ()).expect("2^40 signed");
    let one: u8 = engine.call("script", "one", ()).expect("one");
    let negative: i32 = engine.call("script", "negative", ()).expect("negative");

    assert_eq!((half, narrow), (0.5, 0.5));
    assert_eq!((big, big_signed), (1 << 40, 1 << 40));
    assert_eq!((one, negative), (1, -1));
}

#[test]
fn results_that_do_not_fit_fail_with_a_conversion_error() {
    let engine = engine();

    let fraction = engine.call::<i32>("script", "fraction", ());
    let wide = engine.call::<u8>("script", "wide", ());
    let negative = engine.call::<u32>("script", "negative", ());
    let unsafe_big = engine.call::<i64>("script", "unsafe_big", ());
    let as_bool = engine.call::<bool>("script", "one", ());
    let as_number = engine.call::<f64>("script", "text", ());
    let from_nothing = engine.call::<f64>("script", "nothing", ());

    for (name, result) in [
        ("fraction", fraction.map(|_| ())),
        ("wide", wide.map(|_| ())),
        ("negative", negative.map(|_| ())),
        ("unsafe_big", unsafe_big.map(|_| ())),
        ("as_bool", as_bool.map(|_| ())),
        ("as_number", as_number.map(|_| ())),
        ("from_nothing", from_nothing.map(|_| ())),
    ] {
        let error = result.expect_err(name);
        assert!(
            error.to_string().contains("Error converting from js"),
            "{name}: {error}"
        );
    }
}

#[test]
fn a_function_without_a_result_still_decodes_as_unit() {
    let engine = engine();

    engine
        .call::<()>("script", "nothing", ())
        .expect("undefined is unit");
}

#[test]
fn a_throw_surfaces_with_its_message_and_leaves_the_engine_usable() {
    let engine = engine();

    let error = engine
        .call::<f64>("script", "boom", (3.0,))
        .expect_err("throws");
    let after: f64 = engine.call("script", "sum", (1.0, 2.0)).expect("usable");

    assert!(matches!(error, EngineError::Execution { .. }), "{error:?}");
    assert!(error.to_string().contains("boom 3"), "{error}");
    assert_eq!(after, 3.0);
}

#[test]
fn an_async_export_resolves_before_its_number_decodes_and_one_that_hangs_fails() {
    let engine = engine();

    let later: f64 = engine.call("script", "later", (41.0,)).expect("awaited");
    let never = engine.call::<f64>("script", "never", ());

    assert_eq!(later, 42.0);
    assert!(never.is_err(), "{never:?}");
}
