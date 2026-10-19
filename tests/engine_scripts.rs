//! Loading scripts, calling their exports, and what a script can see of other scripts.

mod support;

use manivelle::EngineError;
use support::{engine, engine_with};

/// An export is read at every call, not remembered: a binding the module reassigns is
/// what the next call runs, and a name that is not a function is not callable.
#[test]
fn a_call_reads_the_export_binding_at_the_time_of_the_call() {
    let engine = engine_with(
        r#"
        export let version = (): number => 1;
        export const answer = 42;
        export function upgrade(): void { version = (): number => 2; }
        "#,
    );

    let first: f64 = engine.call("script", "version", ()).expect("first version");
    engine.call::<()>("script", "upgrade", ()).expect("upgrade");
    let second: f64 = engine
        .call("script", "version", ())
        .expect("second version");
    let not_a_function = engine.call::<f64>("script", "answer", ());
    let missing = engine.call::<f64>("script", "absent", ());

    assert_eq!((first, second), (1.0, 2.0));
    assert!(not_a_function.is_err(), "{not_a_function:?}");
    assert!(
        matches!(missing, Err(EngineError::FunctionNotFound { ref function_name, .. }) if function_name == "absent"),
        "{missing:?}"
    );
}

#[test]
fn calling_an_unknown_script_is_script_not_found() {
    let result = engine().call::<f64>("missing", "run", ());

    assert!(
        matches!(result, Err(EngineError::ScriptNotFound { ref script_id }) if script_id == "missing"),
        "{result:?}"
    );
}

#[test]
fn calling_a_missing_export_is_function_not_found() {
    let engine = engine_with("export function present(): number { return 1; }");

    let result = engine.call::<f64>("script", "absent", ());

    assert!(
        matches!(result, Err(EngineError::FunctionNotFound { ref function_name, .. }) if function_name == "absent"),
        "{result:?}"
    );
}

#[test]
fn script_exceptions_reach_rust_with_their_message() {
    let engine = engine_with(r#"export function run(): void { throw new Error("bad input"); }"#);

    let result = engine.call::<()>("script", "run", ());

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("bad input")),
        "{result:?}"
    );
}

#[test]
fn unloaded_scripts_are_gone() {
    let mut engine = engine_with("export function run(): number { return 1; }");

    engine.unload_script("script").expect("unload script");

    assert!(matches!(
        engine.call::<f64>("script", "run", ()),
        Err(EngineError::ScriptNotFound { .. })
    ));
    assert!(matches!(
        engine.unload_script("script"),
        Err(EngineError::ScriptNotFound { .. })
    ));
}

#[test]
fn scripts_do_not_share_globals() {
    let mut engine = engine();
    let source = r#"
        (globalThis as any).counter = ((globalThis as any).counter ?? 0) + 1;
        export function counter(): number { return (globalThis as any).counter; }
    "#;
    engine.load_script("first", source).expect("load first");
    engine.load_script("second", source).expect("load second");

    let first: f64 = engine.call("first", "counter", ()).expect("first counter");
    let second: f64 = engine
        .call("second", "counter", ())
        .expect("second counter");

    assert_eq!((first, second), (1.0, 1.0));
}

#[test]
fn a_syntax_error_fails_the_load_and_keeps_the_previous_version() {
    let mut engine = engine_with("export function version(): number { return 1; }");

    let reload = engine.load_script("script", "export function version( { return 2; }");
    let version: f64 = engine
        .call("script", "version", ())
        .expect("previous version");

    assert!(
        matches!(reload, Err(EngineError::Transpile { .. })),
        "{reload:?}"
    );
    assert_eq!(version, 1.0);
}

#[test]
fn dynamic_imports_are_rejected_at_load() {
    let result = engine().load_script(
        "script",
        r#"export async function run() { return import("./other"); }"#,
    );

    assert!(result.is_err(), "{result:?}");
}

#[test]
fn call_sees_an_export_the_module_reassigned() {
    let engine = engine_with(
        r#"export let pick = (): number => 1;
           export function swap(): void { pick = () => 2; }"#,
    );

    let before: f64 = engine.call("script", "pick", ()).expect("first pick");
    engine.call::<()>("script", "swap", ()).expect("swap");
    let after: f64 = engine.call("script", "pick", ()).expect("second pick");

    assert_eq!((before, after), (1.0, 2.0));
}
