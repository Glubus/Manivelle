//! A script that runs out of its time or memory while the host delivers to it does not
//! take the other scripts with it, and one that keeps doing so is quarantined.

mod support;

use std::time::Duration;

use manivelle::{Engine, EngineError, EngineOptions};
use serde_json::json;
use support::count;

const SPIN: &str = r#"
import { ctx } from "manivelle:env";
ctx.on("tick", () => { while (true) {} });
export {};
"#;

const COUNTER: &str = r#"
import { ctx } from "manivelle:env";
let ticks = 0;
ctx.on("tick", () => { ticks += 1; });
export const count = (): number => ticks;
"#;

fn engine(max_script_faults: u32) -> Engine {
    Engine::new(&EngineOptions {
        execution_timeout: Duration::from_millis(30),
        max_script_faults,
        ..EngineOptions::default()
    })
    .expect("create engine")
}

fn quarantined(engine: &Engine) -> Vec<&str> {
    engine.quarantined_scripts().collect()
}

#[test]
fn a_runaway_handler_does_not_starve_the_scripts_after_it() {
    let mut engine = engine(3);
    engine.load_script("spin", SPIN).expect("load spin");
    engine.load_script("good", COUNTER).expect("load good");

    let result = engine.emit("tick", &json!({}));

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("script `spin`") && details.contains("fault 1 of 3")),
        "{result:?}"
    );
    assert_eq!(count(&engine, "good"), 1.0);
    assert!(quarantined(&engine).is_empty());
}

#[test]
fn a_script_that_keeps_running_out_of_its_budget_is_quarantined() {
    let mut engine = engine(2);
    engine.load_script("spin", SPIN).expect("load spin");
    engine.load_script("good", COUNTER).expect("load good");

    let first = engine.emit("tick", &json!({}));
    let second = engine.emit("tick", &json!({}));
    let third = engine.emit("tick", &json!({}));

    assert!(first.is_err());
    assert!(
        matches!(&second, Err(EngineError::Execution { details }) if details.contains("quarantined")),
        "{second:?}"
    );
    assert_eq!(third.expect("a quarantined script is no longer visited"), 1);
    assert_eq!(quarantined(&engine), ["spin"]);
    assert_eq!(count(&engine, "good"), 3.0);
}

#[test]
fn the_timers_of_a_quarantined_script_stop_firing() {
    let mut engine = engine(1);
    engine
        .load_script(
            "spin",
            r#"
            import { setInterval } from "manivelle:env";
            setInterval(() => { while (true) {} }, 0);
            export {};
            "#,
        )
        .expect("load");

    let first = engine.advance_timers(Duration::from_millis(1));
    let second = engine.advance_timers(Duration::from_millis(1));

    assert!(first.is_err());
    assert_eq!(second.expect("nothing left to fire"), 0);
    assert_eq!(quarantined(&engine), ["spin"]);
}

#[test]
fn reloading_a_quarantined_script_starts_it_again() {
    let mut engine = engine(1);
    engine.load_script("script", SPIN).expect("load spin");
    let _ = engine.emit("tick", &json!({}));
    assert_eq!(quarantined(&engine), ["script"]);

    engine.load_script("script", COUNTER).expect("reload");
    engine.emit("tick", &json!({})).expect("emit");

    assert!(quarantined(&engine).is_empty());
    assert_eq!(count(&engine, "script"), 1.0);
}

#[test]
fn zero_max_faults_never_quarantines() {
    let mut engine = engine(0);
    engine.load_script("spin", SPIN).expect("load spin");

    for _ in 0..4 {
        let result = engine.emit("tick", &json!({}));
        assert!(
            matches!(&result, Err(EngineError::Execution { details }) if !details.contains("quarantined")),
            "{result:?}"
        );
    }

    assert!(quarantined(&engine).is_empty());
}

#[test]
fn an_ordinary_error_is_not_a_fault() {
    let mut engine = engine(1);
    engine
        .load_script(
            "thrower",
            r#"
            import { ctx } from "manivelle:env";
            ctx.on("tick", () => { throw new Error("just a bug"); });
            export {};
            "#,
        )
        .expect("load");

    for _ in 0..3 {
        assert!(engine.emit("tick", &json!({})).is_err());
    }

    assert!(quarantined(&engine).is_empty());
}

#[test]
fn running_out_of_memory_is_a_fault_that_leaves_the_other_scripts_working() {
    let mut engine = Engine::new(&EngineOptions {
        memory_limit_bytes: 4 * 1024 * 1024,
        max_script_faults: 1,
        ..EngineOptions::default()
    })
    .expect("create engine");
    engine
        .load_script(
            "hog",
            r#"
            import { ctx } from "manivelle:env";
            const keep: number[][] = [];
            ctx.on("tick", () => { for (;;) keep.push(new Array(10000).fill(1)); });
            export {};
            "#,
        )
        .expect("load hog");
    engine.load_script("good", COUNTER).expect("load good");

    let result = engine.emit("tick", &json!({}));

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("script `hog`") && details.contains("out of memory")),
        "{result:?}"
    );
    assert_eq!(quarantined(&engine), ["hog"]);

    // Unloading frees what the hog held, so the others have room again.
    engine.unload_script("hog").expect("unload");
    assert_eq!(engine.emit("tick", &json!({})).expect("emit"), 1);
    assert_eq!(count(&engine, "good"), 2.0);
}

#[test]
fn a_script_patching_a_built_in_reaches_its_group_and_no_other_context() {
    let mut engine = engine(3);
    let patch = "Array.prototype.map = function () { return 'patched'; }; export {};";
    let probe = "export const probe = (): unknown => [1].map((x) => x);";
    engine.load_script_in("pack", "evil", patch).expect("load");
    engine
        .load_script_in("pack", "neighbour", probe)
        .expect("load");
    engine.load_script("outsider", probe).expect("load");

    assert_eq!(
        engine
            .call::<serde_json::Value>("neighbour", "probe", ())
            .expect("call"),
        json!("patched"),
        "a context group is a trust boundary"
    );
    assert_eq!(
        engine
            .call::<serde_json::Value>("outsider", "probe", ())
            .expect("call"),
        json!([1]),
        "a context of its own is not reached"
    );
}
