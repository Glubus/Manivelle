//! Promises and async exports: reactions, settling, rejections and interruption.

mod support;

use manivelle::EngineError;
use support::engine_with;

#[test]
fn promise_reactions_scheduled_by_a_call_run_before_it_returns() {
    let engine = engine_with(
        r#"
        let ticks = 0;
        export function schedule(): void { Promise.resolve().then(() => { ticks += 1; }); }
        export function read(): number { return ticks; }
        "#,
    );

    engine
        .call::<()>("script", "schedule", ())
        .expect("schedule");
    let ticks: f64 = engine.call("script", "read", ()).expect("read ticks");

    assert_eq!(ticks, 1.0);
}

#[test]
fn async_exports_resolve_to_their_value() {
    let engine = engine_with(
        r#"
        export async function answer(): Promise<number> {
            const half = await Promise.resolve(21);
            return half * 2;
        }
        "#,
    );

    let answer: f64 = engine
        .call("script", "answer", ())
        .expect("call async export");

    assert_eq!(answer, 42.0);
}

#[test]
fn a_rejected_async_export_fails_the_call_with_its_reason() {
    let engine = engine_with(
        r#"export async function run(): Promise<void> { throw new Error("async bad input"); }"#,
    );

    let result = engine.call::<()>("script", "run", ());

    assert!(
        matches!(result, Err(EngineError::Execution { ref details })
            if details.contains("async bad input") && !details.contains("unhandled")),
        "{result:?}"
    );
}

#[test]
fn an_async_export_that_can_never_settle_fails_the_call() {
    let engine = engine_with(
        r#"export async function wait(): Promise<void> { await new Promise(() => {}); }"#,
    );

    let result = engine.call::<()>("script", "wait", ());

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("never settles")),
        "{result:?}"
    );
}

#[test]
fn an_unhandled_rejection_fails_the_call() {
    let engine = engine_with(
        r#"export function run(): number { Promise.reject(new Error("lost")); return 1; }"#,
    );

    let result = engine.call::<f64>("script", "run", ());

    assert!(
        matches!(result, Err(EngineError::Execution { ref details })
            if details.contains("unhandled promise rejection") && details.contains("lost")),
        "{result:?}"
    );
}

#[test]
fn a_rejection_handled_before_the_call_ends_is_not_reported() {
    let engine = engine_with(
        r#"
        export function run(): number {
            const rejected = Promise.reject(new Error("recovered"));
            Promise.resolve().then(() => rejected.catch(() => {}));
            return 1;
        }
        "#,
    );

    let result: f64 = engine.call("script", "run", ()).expect("handled rejection");

    assert_eq!(result, 1.0);
}
