//! The operation budget: host time counts against it, and a runaway script is interrupted.

mod support;

use std::time::{Duration, Instant};

use manivelle::{Engine, EngineError, EngineOptions};
use support::Double;

fn engine_with_timeout(timeout: Duration) -> Engine {
    Engine::new(&EngineOptions {
        execution_timeout: timeout,
        ..EngineOptions::default()
    })
    .expect("create engine")
}

#[test]
fn budget_includes_synchronous_host_handler_time() {
    let mut engine = engine_with_timeout(Duration::from_millis(200));
    engine
        .registry()
        .function_with::<Double>(|n| {
            std::thread::sleep(Duration::from_millis(400));
            Ok(n * 2.0)
        })
        .expect("register");
    engine
        .load_script(
            "slow",
            "export function invoke() { return math.double(2); }",
        )
        .expect("load");
    assert!(
        matches!(
            engine.call::<f64>("slow", "invoke", ()),
            Err(EngineError::Execution { .. })
        ),
        "the host handler must consume the operation budget"
    );
}

/// The budget's clock starts at the first interrupt check or host function, so a
/// context whose interrupt counter is not due, a warm one, must still count the time a
/// host function takes.
#[test]
fn host_time_counts_against_the_budget_in_a_warm_context() {
    let mut engine = engine_with_timeout(Duration::from_millis(200));
    engine
        .registry()
        .function_with::<Double>(|n| {
            if n > 0.0 {
                std::thread::sleep(Duration::from_millis(400));
            }
            Ok(n * 2.0)
        })
        .expect("register");
    engine
        .load_script(
            "slow",
            "export function invoke(n: number) { return (globalThis as any).math.double(n); }",
        )
        .expect("load");
    engine
        .call::<f64>("slow", "invoke", (0.0,))
        .expect("a call that does not sleep warms the context");

    let result = engine.call::<f64>("slow", "invoke", (1.0,));

    assert!(
        matches!(result, Err(EngineError::Execution { .. })),
        "the sleeping host call must consume the budget: {result:?}"
    );
}

#[test]
fn runaway_call_is_interrupted_and_engine_stays_usable() {
    let mut engine = engine_with_timeout(Duration::from_millis(100));
    engine
        .load_script(
            "loops",
            r#"
            export function spin(): void { while (true) {} }
            export function add(a: number, b: number): number { return a + b; }
            "#,
        )
        .expect("load script");

    let started = Instant::now();
    let result = engine.call::<()>("loops", "spin", ());

    assert!(result.is_err(), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let sum: f64 = engine
        .call("loops", "add", (2, 3))
        .expect("call after interrupt");
    assert_eq!(sum, 5.0);
}

#[test]
fn runaway_top_level_code_fails_the_load_and_keeps_the_previous_version() {
    let mut engine = engine_with_timeout(Duration::from_millis(100));
    engine
        .load_script("script", "export function version(): number { return 1; }")
        .expect("load first version");

    let reload = engine.load_script(
        "script",
        "while (true) {}\nexport function version(): number { return 2; }",
    );

    assert!(reload.is_err(), "{reload:?}");
    let version: f64 = engine
        .call("script", "version", ())
        .expect("previous version");
    assert_eq!(version, 1.0);
}

#[test]
fn a_runaway_promise_chain_is_interrupted_and_engine_stays_usable() {
    let mut engine = engine_with_timeout(Duration::from_millis(100));
    engine
        .load_script(
            "chain",
            r#"
            function spin(): void { Promise.resolve().then(spin); }
            export function start(): void { spin(); }
            export function add(a: number, b: number): number { return a + b; }
            "#,
        )
        .expect("load script");

    let started = Instant::now();
    let result = engine.call::<()>("chain", "start", ());

    assert!(result.is_err(), "{result:?}");
    assert!(started.elapsed() < Duration::from_secs(5));
    let sum: f64 = engine
        .call("chain", "add", (2, 3))
        .expect("call after interrupt");
    assert_eq!(sum, 5.0);
}
