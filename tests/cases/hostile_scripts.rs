//! What a hostile script cannot do to the host or to the other scripts: values it hands
//! to Rust, memory it allocates outside its heap, and work it hides in Promise jobs.

use std::time::Duration;

use manivelle::{Engine, EngineError, EngineOptions};
use serde_json::{Value, json};

fn engine_with(options: EngineOptions) -> Engine {
    Engine::new(&options).expect("create engine")
}

fn error_text(error: &EngineError) -> String {
    error.to_string()
}

#[test]
fn a_cyclic_value_returned_to_rust_is_an_error_and_not_a_stack_overflow() {
    let mut engine = engine_with(EngineOptions::default());
    engine
        .load_script(
            "cycle",
            "export function go(): unknown { const a: unknown[] = []; a.push(a); return a; }",
        )
        .expect("load");

    let result = engine.call::<Value>("cycle", "go", ());

    let error = result.expect_err("a cycle cannot become JSON");
    assert!(error_text(&error).contains("nested too deeply"), "{error}");
}

#[test]
fn a_value_nested_a_hundred_thousand_levels_deep_is_an_error_too() {
    let mut engine = engine_with(EngineOptions::default());
    engine
        .load_script(
            "deep",
            "export function go(): unknown { let x: unknown = []; for (let i = 0; i < 100000; i++) x = [x]; return x; }",
        )
        .expect("load");

    let result = engine.call::<Value>("deep", "go", ());

    let error = result.expect_err("a value this deep cannot become JSON");
    assert!(error_text(&error).contains("nested too deeply"), "{error}");
    // The engine is usable afterwards.
    engine
        .load_script("ok", "export const one = (): number => 1;")
        .expect("load");
    assert_eq!(engine.call::<f64>("ok", "one", ()).expect("call"), 1.0);
}

#[test]
fn a_value_within_the_depth_limit_still_crosses() {
    let mut engine = engine_with(EngineOptions::default());
    engine
        .load_script(
            "nested",
            "export function go(): unknown { let x: unknown = 1; for (let i = 0; i < 100; i++) x = [x]; return x; }",
        )
        .expect("load");

    let value = engine.call::<Value>("nested", "go", ()).expect("call");

    let mut depth = 0;
    let mut inner = &value;
    while let Value::Array(items) = inner {
        inner = &items[0];
        depth += 1;
    }
    assert_eq!((depth, inner), (100, &json!(1)));
}

#[test]
fn a_property_set_to_undefined_is_absent_from_the_json_the_host_reads() {
    let mut engine = engine_with(EngineOptions::default());
    engine
        .load_script(
            "holes",
            r#"export function go(): unknown {
                return { a: undefined, b: 1, run() {}, inner: { c: undefined }, list: [undefined] };
            }"#,
        )
        .expect("load");

    let value = engine.call::<Value>("holes", "go", ()).expect("call");

    // As `JSON.stringify` writes it: the properties vanish, the array slot becomes `null`.
    assert_eq!(value, json!({ "b": 1, "inner": {}, "list": [null] }));
}

#[test]
fn functions_and_symbols_in_arrays_become_null_and_stay_dropped_as_properties() {
    let mut engine = engine_with(EngineOptions::default());
    engine
        .load_script(
            "items",
            r#"export function list(): unknown { return [1, () => 0, Symbol("s"), undefined]; }
               export function prop(): unknown { return { f: () => 0, s: Symbol("s"), n: 1 }; }
               export function top(): unknown { return () => 0; }"#,
        )
        .expect("load");

    let list = engine.call::<Value>("items", "list", ()).expect("list");
    let prop = engine.call::<Value>("items", "prop", ()).expect("prop");
    let top = engine.call::<Value>("items", "top", ()).expect("top");

    assert_eq!(list, json!([1, null, null, null]));
    assert_eq!(prop, json!({ "n": 1 }));
    assert_eq!(top, json!({}));
}

#[test]
fn a_date_map_or_set_handed_to_rust_as_json_is_refused_and_named() {
    let cases = [
        ("new Date(0)", "Date is not plain data"),
        ("new Map([[1, 2]])", "Map is not plain data"),
        ("new Set([1])", "Set is not plain data"),
        ("{ toJSON() { return 1; } }", "Object is not plain data"),
        ("{ when: new Date(0) }", "[when]: Date is not plain data"),
        ("[1, new Map()]", "[1]: Map is not plain data"),
    ];
    for (source, expected) in cases {
        let mut engine = engine_with(EngineOptions::default());
        engine
            .load_script(
                "shape",
                &format!("export function go(): unknown {{ return {source}; }}"),
            )
            .expect("load");

        let result = engine.call::<Value>("shape", "go", ());

        let error = result.expect_err("JSON would write `{}` or a string for this value");
        assert!(error_text(&error).contains(expected), "{source}: {error}");
    }
}

#[test]
fn text_encoder_output_counts_against_the_memory_limit() {
    let mut engine = engine_with(EngineOptions {
        memory_limit_bytes: 4 * 1024 * 1024,
        ..EngineOptions::default()
    });
    engine
        .load_script(
            "encoder",
            r#"export function go(): number {
                const encoder = new TextEncoder();
                const text = "x".repeat(1 << 20);
                const kept: Uint8Array[] = [];
                for (;;) kept.push(encoder.encode(text));
            }"#,
        )
        .expect("load");

    let result = engine.call::<f64>("encoder", "go", ());

    let error = result.expect_err("the pool must run out");
    assert!(error_text(&error).contains("out of memory"), "{error}");
}

/// The hostile script runs out of this budget; the healthy ones only need to be quicker
/// than it, so it is generous enough that a loaded machine never faults them.
fn budget(max_script_faults: u32) -> EngineOptions {
    EngineOptions {
        execution_timeout: Duration::from_secs(1),
        max_script_faults,
        ..EngineOptions::default()
    }
}

const COUNTER: &str = r#"
import { ctx } from "manivelle:env";
let ticks = 0;
ctx.on("tick", () => { ticks += 1; });
export const count = (): number => ticks;
"#;

#[test]
fn a_loop_hidden_in_a_promise_job_faults_the_script_that_queued_it() {
    let mut engine = engine_with(budget(2));
    engine
        .load_script(
            "sneaky",
            r#"import { ctx } from "manivelle:env";
               ctx.on("tick", () => { Promise.resolve().then(() => { while (true) {} }); });
               export {};"#,
        )
        .expect("load sneaky");
    engine.load_script("good", COUNTER).expect("load good");

    let first = engine.emit("tick", &json!({}));
    let second = engine.emit("tick", &json!({}));
    let third = engine.emit("tick", &json!({}));

    assert!(
        matches!(&first, Err(EngineError::Execution { details }) if details.contains("script `sneaky`")),
        "{first:?}"
    );
    assert!(
        matches!(&second, Err(EngineError::Execution { details }) if details.contains("quarantined")),
        "{second:?}"
    );
    assert_eq!(third.expect("the quarantined script is not visited"), 1);
    assert_eq!(engine.quarantined_scripts().collect::<Vec<_>>(), ["sneaky"]);
    assert_eq!(engine.call::<f64>("good", "count", ()).expect("count"), 3.0);
}

#[test]
fn a_job_one_script_queued_is_not_charged_to_the_script_after_it() {
    let mut engine = engine_with(budget(1));
    engine
        .load_script(
            "queuer",
            r#"import { ctx } from "manivelle:env";
               ctx.on("tick", () => { Promise.resolve().then(() => { while (true) {} }); });
               export {};"#,
        )
        .expect("load queuer");
    engine
        .load_script(
            "async-victim",
            r#"import { ctx } from "manivelle:env";
               ctx.on("tick", async () => { await Promise.resolve(); return 1; });
               export {};"#,
        )
        .expect("load victim");

    let _ = engine.request::<_, f64>("tick", &json!({}));

    assert_eq!(
        engine.quarantined_scripts().collect::<Vec<_>>(),
        ["queuer"],
        "only the script that queued the spinning job is at fault"
    );
}

#[test]
fn only_the_first_script_to_run_out_of_memory_in_an_operation_is_counted() {
    let mut engine = engine_with(EngineOptions {
        memory_limit_bytes: 4 * 1024 * 1024,
        max_script_faults: 1,
        ..EngineOptions::default()
    });
    // Fills the pool and swallows the error, so it is not itself a fault.
    engine
        .load_script(
            "hog",
            r#"import { ctx } from "manivelle:env";
               const kept: number[][] = [];
               ctx.on("tick", () => {
                   try { for (;;) kept.push(new Array(1000).fill(1)); } catch {}
               });
               export {};"#,
        )
        .expect("load hog");
    let allocating = r#"import { ctx } from "manivelle:env";
        ctx.on("tick", () => { const a: number[] = new Array(200000).fill(1); return a.length; });
        export {};"#;
    engine.load_script("v1", allocating).expect("load v1");
    engine.load_script("v2", allocating).expect("load v2");

    let result = engine.emit("tick", &json!({}));

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("script `v1`") && details.contains("ran out of memory")),
        "{result:?}"
    );
    assert_eq!(
        engine.quarantined_scripts().collect::<Vec<_>>(),
        ["v1"],
        "only the first script to run out of memory is at fault, not v2 or the hog"
    );
}

#[test]
fn running_out_of_memory_inside_a_context_group_is_judged_without_a_second_runtime_lock() {
    let mut engine = engine_with(EngineOptions {
        memory_limit_bytes: 4 * 1024 * 1024,
        max_script_faults: 1,
        ..EngineOptions::default()
    });
    // Each script allocates until the pool is gone, with and without catching the error,
    // so the failure reaches the engine as the several messages QuickJS can give.
    let sources = [
        r#"import { ctx } from "manivelle:env";
           const kept: number[][] = [];
           ctx.on("tick", () => { for (;;) kept.push(new Array(1000).fill(1)); });
           export {};"#,
        r#"import { ctx } from "manivelle:env";
           const kept: string[] = [];
           ctx.on("tick", () => { for (let i = 0; ; i++) kept.push("x".repeat(100000) + i); });
           export {};"#,
        r#"import { ctx } from "manivelle:env";
           ctx.on("tick", () => { const o: Record<string, number> = {}; for (let i = 0; ; i++) o["k" + i] = i; });
           export {};"#,
    ];
    for (index, source) in sources.iter().enumerate() {
        engine
            .load_script_in("pack", format!("hog-{index}"), source)
            .expect("load");
    }

    for (index, _) in sources.iter().enumerate() {
        let result = engine.emit("tick", &json!({}));

        let name = format!("script `hog-{index}`");
        assert!(
            matches!(&result, Err(EngineError::Execution { details })
                if details.contains(&name) && details.contains("ran out of memory")),
            "{result:?}"
        );
    }

    assert_eq!(
        engine.quarantined_scripts().collect::<Vec<_>>(),
        ["hog-0", "hog-1", "hog-2"]
    );
}
