use std::time::{Duration, Instant};

use manivelle::{
    Engine, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
    VmContractValidation, VmError, VmOptions,
};
use serde_json::json;

fn engine_with_timeout(timeout: Duration) -> Engine {
    Engine::new(&VmOptions {
        execution_timeout: timeout,
        ..VmOptions::default()
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
            Err(VmError::Execution { .. })
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
        matches!(result, Err(VmError::Execution { .. })),
        "the sleeping host call must consume the budget: {result:?}"
    );
}

/// A delivery in progress runs the handler list it took: a handler removed meanwhile
/// still runs in it, one added meanwhile waits for the next delivery.
#[test]
fn a_delivery_runs_the_snapshot_it_took_even_when_handlers_change_the_list() {
    let engine = engine_with(
        r#"
        const order: string[] = [];
        const late = () => { order.push("late"); };
        const second = (event: { n: number }) => { order.push(`second:${event.n}`); };
        const first = (event: { n: number }) => {
            order.push(`first:${event.n}`);
            ctx.off("tick", second);
            ctx.on("tick", late);
        };
        ctx.on("tick", first);
        ctx.on("tick", second);
        export function read(): string { return order.join(","); }
        export function clear(): void { order.length = 0; }
        "#,
    );

    engine
        .emit("tick", &json!({ "n": 7 }))
        .expect("first delivery");
    let first: String = engine.call("script", "read", ()).expect("read");
    engine.call::<()>("script", "clear", ()).expect("clear");
    engine
        .emit("tick", &json!({ "n": 8 }))
        .expect("second delivery");
    let second: String = engine.call("script", "read", ()).expect("read");

    assert_eq!(
        first, "first:7,second:7",
        "`second` was still in the list taken"
    );
    assert_eq!(
        second, "first:8,late",
        "the next delivery sees the new list"
    );
}

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
        matches!(missing, Err(VmError::FunctionNotFound { ref function_name, .. }) if function_name == "absent"),
        "{missing:?}"
    );
}

fn engine() -> Engine {
    Engine::new(&VmOptions::default()).expect("create engine")
}

fn engine_with(source: &str) -> Engine {
    let mut engine = engine();
    engine.load_script("script", source).expect("load script");
    engine
}

struct Double;

impl HostContract for Double {
    const NAME: &'static str = "math.double";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["math", "double"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Double {
    type Input = f64;
    type Output = f64;
}

impl HostFunction for Double {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(input * 2.0)
    }
}

struct Failing;

impl HostContract for Failing {
    const NAME: &'static str = "host.fail";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Failing {
    type Input = ();
    type Output = f64;
}

impl HostFunction for Failing {
    fn call((): Self::Input) -> Result<Self::Output, VmError> {
        Err(VmError::Execution {
            details: String::from("boom"),
        })
    }
}

#[test]
fn calling_an_unknown_script_is_script_not_found() {
    let result = engine().call::<f64>("missing", "run", ());

    assert!(
        matches!(result, Err(VmError::ScriptNotFound { ref script_id }) if script_id == "missing"),
        "{result:?}"
    );
}

#[test]
fn calling_a_missing_export_is_function_not_found() {
    let engine = engine_with("export function present(): number { return 1; }");

    let result = engine.call::<f64>("script", "absent", ());

    assert!(
        matches!(result, Err(VmError::FunctionNotFound { ref function_name, .. }) if function_name == "absent"),
        "{result:?}"
    );
}

#[test]
fn script_exceptions_reach_rust_with_their_message() {
    let engine = engine_with(r#"export function run(): void { throw new Error("bad input"); }"#);

    let result = engine.call::<()>("script", "run", ());

    assert!(
        matches!(result, Err(VmError::Execution { ref details }) if details.contains("bad input")),
        "{result:?}"
    );
}

#[test]
fn a_throwing_event_handler_fails_the_emit() {
    let engine =
        engine_with(r#"ctx.on("tick", () => { throw new Error("handler failed"); }); export {};"#);

    let result = engine.emit("tick", &json!({}));

    assert!(
        matches!(result, Err(VmError::Execution { ref details }) if details.contains("handler failed")),
        "{result:?}"
    );
}

#[test]
fn unloaded_scripts_are_gone() {
    let mut engine = engine_with("export function run(): number { return 1; }");

    engine.unload_script("script").expect("unload script");

    assert!(matches!(
        engine.call::<f64>("script", "run", ()),
        Err(VmError::ScriptNotFound { .. })
    ));
    assert!(matches!(
        engine.unload_script("script"),
        Err(VmError::ScriptNotFound { .. })
    ));
}

#[test]
fn typed_host_functions_are_reachable_by_import_and_by_global() {
    let mut engine = engine();
    engine
        .registry()
        .function::<Double>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"
            import { math as imported } from "host";
            export function viaImport(value: number): number { return imported.double(value); }
            export function viaGlobal(value: number): number { return (globalThis as any).math.double(value); }
            "#,
        )
        .expect("load script");

    let via_import: f64 = engine
        .call("script", "viaImport", (21,))
        .expect("call via import");
    let via_global: f64 = engine
        .call("script", "viaGlobal", (4,))
        .expect("call via global");

    assert_eq!(via_import, 42.0);
    assert_eq!(via_global, 8.0);
}

#[test]
fn untyped_host_functions_use_the_json_path() {
    let mut engine = engine();
    engine
        .registry()
        .function::<Double>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"export function run(): number { return (globalThis as any).math.double(2.5); }"#,
        )
        .expect("load script");

    let result: f64 = engine.call("script", "run", ()).expect("call");

    assert_eq!(result, 5.0);
}

#[test]
fn host_function_errors_are_catchable_in_scripts() {
    let mut engine = engine();
    engine
        .registry()
        .function::<Failing>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"
            export function caught(): string {
              try { (globalThis as any).host.fail(); return "no error"; }
              catch (error) { return String((error as Error).message); }
            }
            export function uncaught(): number { return (globalThis as any).host.fail(); }
            "#,
        )
        .expect("load script");

    let caught: String = engine.call("script", "caught", ()).expect("call caught");
    let uncaught = engine.call::<f64>("script", "uncaught", ());

    assert!(caught.contains("boom"), "{caught}");
    assert!(
        matches!(uncaught, Err(VmError::Execution { ref details }) if details.contains("boom")),
        "{uncaught:?}"
    );
}

#[test]
fn contract_validation_rejects_inputs_outside_the_schema() {
    let mut engine = Engine::new(&VmOptions {
        contract_validation: VmContractValidation::Inputs,
        ..VmOptions::default()
    })
    .expect("create engine");
    engine
        .registry()
        .function::<Double>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"
            export function valid(): number { return (globalThis as any).math.double(3); }
            export function invalid(): number { return (globalThis as any).math.double("three"); }
            "#,
        )
        .expect("load script");

    let valid: f64 = engine.call("script", "valid", ()).expect("valid input");
    let invalid = engine.call::<f64>("script", "invalid", ());

    assert_eq!(valid, 6.0);
    assert!(
        matches!(invalid, Err(VmError::Execution { ref details }) if details.contains("validation")),
        "{invalid:?}"
    );
}

#[test]
fn loading_scripts_past_the_memory_limit_says_the_limit_was_reached() {
    let mut engine = Engine::new(&VmOptions {
        memory_limit_bytes: 4 * 1024 * 1024,
        ..VmOptions::default()
    })
    .expect("create engine");
    engine
        .load_script("first", "export function one(): number { return 1; }")
        .expect("load first script");

    let error = (0..1000)
        .find_map(|index| {
            engine
                .load_script(format!("script-{index}"), "export {};")
                .err()
        })
        .expect("4 MiB hold fewer than 1000 contexts");

    assert!(
        matches!(&error, VmError::Execution { details } if details.contains("out of memory") && details.contains("memory_limit_bytes")),
        "{error:?}"
    );
    let one: f64 = engine
        .call("first", "one", ())
        .expect("engine stays usable");
    assert_eq!(one, 1.0);
}

#[test]
fn exceeding_the_memory_limit_fails_the_call_and_the_engine_stays_usable() {
    let mut engine = Engine::new(&VmOptions {
        memory_limit_bytes: 8 * 1024 * 1024,
        ..VmOptions::default()
    })
    .expect("create engine");
    engine
        .load_script(
            "script",
            r#"
            export function hog(): number {
              const chunks: string[] = [];
              for (let i = 0; i < 1_000_000; i++) chunks.push("x".repeat(1024) + i);
              return chunks.length;
            }
            export function small(): number { return 1; }
            "#,
        )
        .expect("load script");

    let hog = engine.call::<f64>("script", "hog", ());
    let small: f64 = engine
        .call("script", "small", ())
        .expect("call after out of memory");

    assert!(hog.is_err(), "{hog:?}");
    assert_eq!(small, 1.0);
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
        matches!(reload, Err(VmError::Transpile { .. })),
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

/// Makes `count` two-object cycles that reference counting alone cannot free.
const CYCLES: &str = r#"
export function makeCycles(count: number): void {
  for (let i = 0; i < count; i++) {
    const a: { b?: unknown } = {};
    a.b = { a };
  }
}
"#;

#[test]
fn cycles_stay_until_run_gc_when_automatic_collection_is_off() {
    let engine = engine_with(CYCLES);
    engine.set_gc_threshold(None);
    let before = engine.memory_stats().object_count;

    engine
        .call::<()>("script", "makeCycles", (20_000,))
        .expect("make cycles");
    let kept = engine.memory_stats().object_count;
    engine.run_gc();
    let collected = engine.memory_stats().object_count;

    assert!(kept >= before + 40_000, "{before} -> {kept}");
    assert!(collected <= before + 100, "{before} -> {collected}");
}

#[test]
fn automatic_collection_resumes_once_a_threshold_is_set_again() {
    let engine = engine_with(CYCLES);
    engine.set_gc_threshold(None);
    engine.set_gc_threshold(Some(64 * 1024));
    let before = engine.memory_stats().object_count;

    engine
        .call::<()>("script", "makeCycles", (20_000,))
        .expect("make cycles");
    let after = engine.memory_stats().object_count;

    assert!(after < before + 20_000, "{before} -> {after}");
}

#[test]
fn emit_calls_every_registered_handler() {
    let mut engine = Engine::new(&VmOptions::default()).expect("create engine");
    engine
        .load_script(
            "listener",
            r#"
            let total = 0;
            ctx.on("tick", event => { total += event.n; });
            ctx.on("tick", event => { total += event.n * 10; });
            export function read(): number { return total; }
            "#,
        )
        .expect("load script");

    let delivered = engine.emit("tick", &json!({ "n": 2 })).expect("emit");
    let total: f64 = engine.call("listener", "read", ()).expect("read total");

    assert_eq!(delivered, 1);
    assert_eq!(total, 22.0);
}

#[test]
fn a_handler_registered_by_a_later_call_receives_events() {
    let engine = engine_with(
        r#"
        let received = 0;
        export function subscribe(): void { ctx.on("tick", () => { received += 1; }); }
        export function read(): number { return received; }
        "#,
    );

    let before = engine
        .emit("tick", &json!(null))
        .expect("emit before subscribing");
    engine
        .call::<()>("script", "subscribe", ())
        .expect("subscribe");
    let after = engine
        .emit("tick", &json!(null))
        .expect("emit after subscribing");
    let received: f64 = engine.call("script", "read", ()).expect("read count");

    assert_eq!((before, after, received), (0, 1, 1.0));
}

#[test]
fn a_reload_without_handlers_stops_receiving_events() {
    let mut engine = engine_with(r#"ctx.on("tick", () => {}); export {};"#);
    engine
        .load_script("script", "export function idle(): void {}")
        .expect("reload without handlers");

    let delivered = engine.emit("tick", &json!(null)).expect("emit");

    assert_eq!(delivered, 0);
}

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
        matches!(result, Err(VmError::Execution { ref details })
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
        matches!(result, Err(VmError::Execution { ref details }) if details.contains("never settles")),
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
        matches!(result, Err(VmError::Execution { ref details })
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

#[test]
fn a_throwing_handler_does_not_stop_delivery_to_other_handlers() {
    let mut engine = engine();
    let recorder = r#"
        let received = 0;
        ctx.on("tick", () => { received += 1; });
        export function read(): number { return received; }
    "#;
    engine
        .load_script(
            "thrower",
            &format!(
                r#"ctx.on("tick", () => {{ throw new Error("handler failed"); }});{recorder}"#
            ),
        )
        .expect("load thrower");
    engine
        .load_script("recorder", recorder)
        .expect("load recorder");

    let result = engine.emit("tick", &json!(null));
    let thrower_received: f64 = engine.call("thrower", "read", ()).expect("read thrower");
    let recorder_received: f64 = engine.call("recorder", "read", ()).expect("read recorder");

    assert!(
        matches!(result, Err(VmError::Execution { ref details }) if details.contains("handler failed")),
        "{result:?}"
    );
    assert_eq!(thrower_received, 1.0);
    assert_eq!(recorder_received, 1.0);
}

#[test]
fn emit_reports_the_first_failure_in_load_order() {
    let mut engine = engine();
    for name in ["zeta", "alpha", "mid", "beta", "omega", "gamma"] {
        engine
            .load_script(
                name,
                &format!(
                    r#"ctx.on("tick", () => {{ throw new Error("from {name}"); }}); export {{}};"#
                ),
            )
            .expect("load script");
    }
    engine
        .load_script(
            "zeta",
            r#"ctx.on("tick", () => { throw new Error("from zeta v2"); }); export {};"#,
        )
        .expect("reload keeps load position");

    let result = engine.emit("tick", &json!(null));

    assert!(
        matches!(result, Err(VmError::Execution { ref details }) if details.contains("from zeta v2")),
        "{result:?}"
    );
}
