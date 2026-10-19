//! The lifecycle of the scripts of a context group: unloading, reloading, failed loads, project
//! loads and the release of memory once a group is empty.

mod support;

use serde_json::json;
use support::{COUNTER, TestCacheDir, count, group_engine};

#[test]
fn unloading_one_script_leaves_the_others_of_its_group_running() {
    let mut engine = group_engine();
    engine
        .load_script_in("pack", "stays", COUNTER)
        .expect("load");
    engine
        .load_script_in("pack", "goes", COUNTER)
        .expect("load");
    engine.emit("frame", &json!({})).expect("emit");

    engine.unload_script("goes").expect("unload");
    engine.emit("frame", &json!({})).expect("emit");

    assert_eq!(count(&engine, "stays"), 2.0);
    assert!(engine.call::<f64>("goes", "count", ()).is_err());
    assert_eq!(
        engine.call::<String>("stays", "me", ()).expect("host call"),
        "stays"
    );
}

#[test]
fn a_group_is_created_again_after_its_last_script_leaves() {
    let mut engine = group_engine();
    engine
        .load_script_in("pack", "first", COUNTER)
        .expect("load");
    engine.unload_script("first").expect("unload");

    engine
        .load_script_in("pack", "second", COUNTER)
        .expect("load again");
    engine.emit("frame", &json!({})).expect("emit");

    assert_eq!(count(&engine, "second"), 1.0);
}

#[test]
fn reloading_a_script_hands_over_its_state_and_leaves_the_others_alone() {
    let mut engine = group_engine();
    let versioned = |version: u32| {
        format!(
            r#"
            import {{ ctx }} from "manivelle:env";
            let frames = ctx.hot.data?.frames ?? 0;
            ctx.hot.save(() => ({{ frames }}));
            ctx.on("frame", () => {{ frames += 1; }});
            export function count(): number {{ return frames; }}
            export function version(): number {{ return {version}; }}
            "#
        )
    };
    engine
        .load_script_in("pack", "hot", &versioned(1))
        .expect("load v1");
    engine
        .load_script_in("pack", "bystander", COUNTER)
        .expect("load");
    engine.emit("frame", &json!({})).expect("emit");

    engine
        .load_script_in("pack", "hot", &versioned(2))
        .expect("reload v2");
    engine.emit("frame", &json!({})).expect("emit");

    assert_eq!(engine.call::<f64>("hot", "version", ()).expect("call"), 2.0);
    assert_eq!(
        count(&engine, "hot"),
        2.0,
        "the new version continues the saved count"
    );
    assert_eq!(
        count(&engine, "bystander"),
        2.0,
        "the bystander never restarted"
    );
}

#[test]
fn a_failed_load_keeps_the_previous_version_in_the_group() {
    let mut engine = group_engine();
    engine
        .load_script_in("pack", "kept", COUNTER)
        .expect("load");
    engine.emit("frame", &json!({})).expect("emit");

    let failed = engine.load_script_in(
        "pack",
        "kept",
        r#"import { ctx } from "manivelle:env"; throw new Error("broken"); export {};"#,
    );
    engine.emit("frame", &json!({})).expect("emit");

    assert!(failed.is_err());
    assert_eq!(count(&engine, "kept"), 2.0);
}

#[test]
fn a_project_loads_into_a_group_and_reloads_in_it() {
    let dir = TestCacheDir::new("group-project");
    let root = dir.path();
    std::fs::write(
        root.join("main.ts"),
        r#"
        import { ctx } from "manivelle:env";
        import { step } from "./step";
        let total = 0;
        ctx.on("frame", () => { total += step; });
        export function read(): number { return total; }
        "#,
    )
    .expect("write main");
    std::fs::write(root.join("step.ts"), "export const step = 1;").expect("write step");
    let mut engine = group_engine();
    engine
        .load_script_in("pack", "other", COUNTER)
        .expect("load other");
    engine
        .load_project_in("pack", "project", root.join("main.ts"))
        .expect("load project");
    engine.emit("frame", &json!({})).expect("emit");

    std::fs::write(root.join("step.ts"), "export const step = 10;").expect("edit step");
    let report = engine.reload_changed();
    engine.emit("frame", &json!({})).expect("emit");

    assert_eq!(report.reloaded, ["project"]);
    assert_eq!(
        engine.call::<f64>("project", "read", ()).expect("read"),
        10.0
    );
    assert_eq!(count(&engine, "other"), 2.0);
}

#[test]
fn unloading_every_script_of_a_group_releases_its_memory() {
    let mut engine = group_engine();
    // rquickjs keeps the first context a runtime creates alive for good, through the
    // prototype it caches for native functions: make that one a throwaway.
    engine.load_script("warm-up", "export {};").expect("load");
    engine.unload_script("warm-up").expect("unload");
    engine.run_gc();
    let before = engine.memory_stats().memory_used_bytes;

    for i in 0..20 {
        engine
            .load_script_in("pack", format!("s{i}"), COUNTER)
            .expect("load");
    }
    let loaded = engine.memory_stats().memory_used_bytes;
    for i in 0..20 {
        engine.unload_script(&format!("s{i}")).expect("unload");
    }
    engine.run_gc();
    let after = engine.memory_stats().memory_used_bytes;

    assert!(loaded > before + 20 * 1024, "twenty scripts take memory");
    assert!(
        after < before + 64 * 1024,
        "before {before}, loaded {loaded}, after {after}"
    );
}
