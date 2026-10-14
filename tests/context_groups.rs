//! Context groups: scripts loaded with `load_script_in` / `load_project_in` under one
//! group name share a QuickJS context (cheaper to load, smaller, faster to deliver to at
//! hundreds of scripts) but keep their own events, timers, console, host-function
//! identity, `ctx.hot` state and lifecycle. They reach those through
//! `import { ... } from "manivelle:env"`, since a shared context has no per-script globals.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use manivelle::{
    ConsoleLevel, Engine, EngineOptions, HostCallback, HostContract, HostContractKind,
    HostFunctionSignature, HostResolver,
};
use serde_json::json;
use support::TestCacheDir;

/// `import { whoami } from "host"`: the id of the calling script.
struct WhoAmI;

impl HostContract for WhoAmI {
    const NAME: &'static str = "host.whoami";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["whoami"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for WhoAmI {
    type Input = ();
    type Output = String;
}

/// `import { lookup } from "host"`: a score, answered by the host later.
struct Lookup;

impl HostContract for Lookup {
    const NAME: &'static str = "host.lookup";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["lookup"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Lookup {
    type Input = ();
    type Output = f64;
}

/// `import { score } from "host"`; `score.onUpdate(handler)` listens to `score.update`.
struct ScoreUpdate;

impl HostContract for ScoreUpdate {
    const NAME: &'static str = "score.update";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["score", "onUpdate"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for ScoreUpdate {
    type Payload = u32;
}

/// Counts `frame` events, tells who it is, and exposes both.
const COUNTER: &str = r#"
import { ctx } from "manivelle:env";
import { whoami } from "host";

let frames = 0;
ctx.on("frame", () => { frames += 1; });
export function count(): number { return frames; }
export function me(): string { return whoami(); }
"#;

fn engine() -> Engine {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with_caller::<WhoAmI>(|caller, ()| Ok(caller.script_id().to_owned()))
        .and_then(|registry| registry.callback::<ScoreUpdate>())
        .expect("register contracts");
    engine.set_console(|_, _, _| {});
    engine
}

fn count(engine: &Engine, id: &str) -> f64 {
    engine.call(id, "count", ()).expect("count")
}

#[test]
fn a_group_shares_one_realm_and_nothing_else_does() {
    let mut engine = engine();
    let writer = "export function write(): void { (globalThis as any).shared = 'set'; }";
    let reader = "export function read(): unknown { return (globalThis as any).shared; }";
    engine
        .load_script_in("pack", "writer", writer)
        .expect("load");
    engine
        .load_script_in("pack", "reader", reader)
        .expect("load");
    engine
        .load_script_in("other", "outsider", reader)
        .expect("load");
    engine.load_script("alone", reader).expect("load");

    engine.call::<()>("writer", "write", ()).expect("write");

    let same: Option<String> = engine.call("reader", "read", ()).expect("read");
    let other: Option<String> = engine.call("outsider", "read", ()).expect("read");
    assert_eq!(
        same.as_deref(),
        Some("set"),
        "scripts of one group share globals"
    );
    assert_eq!(other, None, "another group has its own realm");
}

#[test]
fn a_script_reads_its_environment_the_same_way_in_a_group_or_not() {
    let mut engine = engine();

    engine
        .load_script_in("pack", "grouped", COUNTER)
        .expect("load grouped");
    engine.load_script("alone", COUNTER).expect("load alone");
    engine.emit("frame", &json!({})).expect("emit");

    assert_eq!(count(&engine, "grouped"), 1.0);
    assert_eq!(count(&engine, "alone"), 1.0);
}

#[test]
fn events_reach_every_script_in_load_order_whatever_its_group() {
    let mut engine = engine();
    for (group, id) in [("a", "one"), ("b", "two"), ("a", "three")] {
        engine
            .load_script_in(
                group,
                id,
                &format!(
                    r#"
                    import {{ ctx }} from "manivelle:env";
                    ctx.on("tick", () => "{id}");
                    export {{}};
                    "#
                ),
            )
            .expect("load script");
    }

    let replies: Vec<(&str, String)> = engine.request("tick", &json!({})).expect("request");

    assert_eq!(
        replies,
        [
            ("one", String::from("one")),
            ("two", String::from("two")),
            ("three", String::from("three"))
        ]
    );
}

#[test]
fn a_callback_registered_through_a_host_module_belongs_to_the_importing_script() {
    let mut engine = engine();
    for id in ["first", "second"] {
        engine
            .load_script_in(
                "pack",
                id,
                &format!(
                    r#"
                    import {{ score }} from "host";
                    let seen = 0;
                    score.onUpdate((combo: number) => {{ seen += combo * ("{id}" === "first" ? 1 : 10); }});
                    export function read(): number {{ return seen; }}
                    "#
                ),
            )
            .expect("load script");
    }

    engine.emit("score.update", &3u32).expect("emit");

    assert_eq!(engine.call::<f64>("first", "read", ()).expect("read"), 3.0);
    assert_eq!(
        engine.call::<f64>("second", "read", ()).expect("read"),
        30.0
    );
}

#[test]
fn host_functions_know_which_script_of_the_group_called() {
    let mut engine = engine();
    engine
        .load_script_in("pack", "alpha", COUNTER)
        .expect("load");
    engine
        .load_script_in("pack", "beta", COUNTER)
        .expect("load");

    let alpha: String = engine.call("alpha", "me", ()).expect("call");
    let beta: String = engine.call("beta", "me", ()).expect("call");
    engine
        .load_script_in("pack", "alpha", COUNTER)
        .expect("reload");
    let reloaded: String = engine.call("alpha", "me", ()).expect("call");

    assert_eq!([alpha, beta, reloaded], ["alpha", "beta", "alpha"]);
}

#[test]
fn async_host_calls_settle_the_promise_of_the_script_that_made_them() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    type Pending = Arc<Mutex<Vec<(String, HostResolver<f64>)>>>;
    let pending: Pending = Arc::default();
    let queue = Arc::clone(&pending);
    engine
        .registry()
        .async_function_with_caller::<Lookup>(move |caller, (), resolver| {
            queue
                .lock()
                .expect("pending")
                .push((caller.script_id().to_owned(), resolver));
            Ok(())
        })
        .expect("register");
    for id in ["first", "second"] {
        engine
            .load_script_in(
                "pack",
                id,
                r#"
                import { lookup } from "host";
                export async function score(): Promise<number> { return await lookup() * 2; }
                "#,
            )
            .expect("load script");
    }
    let first = engine
        .call_deferred::<f64>("first", "score", ())
        .expect("start");
    let second = engine
        .call_deferred::<f64>("second", "score", ())
        .expect("start");

    for (script, resolver) in pending.lock().expect("pending").drain(..) {
        resolver
            .resolve(if script == "first" { 1.0 } else { 20.0 })
            .expect("resolve");
    }
    engine.pump().expect("pump");

    assert_eq!(first.take().expect("finished").expect("value"), 2.0);
    assert_eq!(second.take().expect("finished").expect("value"), 40.0);
}

#[test]
fn console_output_names_the_script_that_logged() {
    let mut engine = engine();
    let lines = Rc::new(RefCell::new(Vec::new()));
    let sink = Rc::clone(&lines);
    engine.set_console(move |level, script, message| {
        assert_eq!(level, ConsoleLevel::Warn);
        sink.borrow_mut()
            .push((script.to_owned(), message.to_owned()));
    });
    for id in ["first", "second"] {
        engine
            .load_script_in(
                "pack",
                id,
                r#"
                import { console } from "manivelle:env";
                export function shout(word: string): void { console.warn(word, 1); }
                "#,
            )
            .expect("load script");
    }

    engine.call::<()>("second", "shout", ("b",)).expect("call");
    engine.call::<()>("first", "shout", ("a",)).expect("call");

    assert_eq!(
        *lines.borrow(),
        [
            (String::from("second"), String::from("b 1")),
            (String::from("first"), String::from("a 1"))
        ]
    );
}

#[test]
fn timers_belong_to_the_script_that_set_them() {
    let mut engine = engine();
    let ticker = |step: u32| {
        format!(
            r#"
            import {{ setInterval, clearInterval }} from "manivelle:env";
            let ticks = 0;
            const timer = setInterval(() => {{ ticks += {step}; }}, 10);
            export function count(): number {{ return ticks; }}
            export function stop(): void {{ clearInterval(timer); }}
            "#
        )
    };
    engine
        .load_script_in("pack", "slow", &ticker(1))
        .expect("load");
    engine
        .load_script_in("pack", "fast", &ticker(100))
        .expect("load");

    engine
        .advance_timers(Duration::from_millis(10))
        .expect("advance");
    engine.call::<()>("fast", "stop", ()).expect("stop");
    engine
        .advance_timers(Duration::from_millis(10))
        .expect("advance");
    engine
        .load_script_in("pack", "slow", &ticker(1))
        .expect("reload");
    engine
        .advance_timers(Duration::from_millis(10))
        .expect("advance");

    assert_eq!(
        count(&engine, "fast"),
        100.0,
        "stopped after its first tick"
    );
    assert_eq!(
        count(&engine, "slow"),
        1.0,
        "a reload starts the script's timers anew"
    );
}

#[test]
fn unloading_one_script_leaves_the_others_of_its_group_running() {
    let mut engine = engine();
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
    let mut engine = engine();
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
    let mut engine = engine();
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
    let mut engine = engine();
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
    let mut engine = engine();
    engine
        .load_script_in("pack", "other", COUNTER)
        .expect("load other");
    engine
        .load_project_in("pack", "project", root.join("main.ts"))
        .expect("load project");
    engine.emit("frame", &json!({})).expect("emit");

    std::thread::sleep(Duration::from_millis(20));
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
    let mut engine = engine();
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

#[test]
fn generated_declarations_type_the_environment_module() {
    let dir = TestCacheDir::new("group-env-tsc");
    let engine = engine();
    std::fs::write(
        dir.path().join("manivelle.d.ts"),
        engine.registry().dts().expect("render declarations"),
    )
    .expect("write declarations");
    let script = dir.path().join("main.ts");
    std::fs::write(
        &script,
        r#"
        /// <reference path="./manivelle.d.ts" />
        import { ctx, console, setTimeout, setInterval, clearInterval } from "manivelle:env";

        ctx.on("score.update", (combo: number) => { console.log(combo.toFixed(0)); });
        ctx.hot.save(() => ({ at: 1 }));
        const timer = setInterval(() => {}, 10);
        clearInterval(timer);
        setTimeout(() => console.warn("late", 1), 5);
        export {};
        "#,
    )
    .expect("write script");

    let Some(output) = support::run_tsc(&script) else {
        return;
    };

    assert!(
        output.status.success(),
        "tsc rejected a script importing manivelle:env\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_scripts_of_a_group_are_handed_one_payload_object_and_other_scripts_their_own() {
    let mut engine = engine();
    let mutating = r#"
        import { ctx } from "manivelle:env";
        ctx.on("tick", (event: { n: number }) => { event.n += 1; return event.n; });
        export {};
    "#;
    engine
        .load_script_in("pack", "first", mutating)
        .expect("load");
    engine
        .load_script_in("pack", "second", mutating)
        .expect("load");
    engine
        .load_script_in("elsewhere", "third", mutating)
        .expect("load");
    engine.load_script("fourth", mutating).expect("load");

    let replies: Vec<(&str, u32)> = engine.request("tick", &json!({ "n": 0 })).expect("request");

    assert_eq!(
        replies,
        [("first", 1), ("second", 2), ("third", 1), ("fourth", 1)],
        "a group's scripts share the event they are handed"
    );
}

#[test]
fn a_script_of_another_group_in_between_does_not_break_the_load_order() {
    let mut engine = engine();
    let listener = |id: &str| {
        format!(
            r#"
            import {{ ctx }} from "manivelle:env";
            ctx.on("tick", () => "{id}");
            export {{}};
            "#
        )
    };
    for (group, id) in [
        ("a", "a1"),
        ("b", "b1"),
        ("a", "a2"),
        ("a", "a3"),
        ("b", "b2"),
    ] {
        engine
            .load_script_in(group, id, &listener(id))
            .expect("load");
    }

    let replies: Vec<(&str, String)> = engine.request("tick", &json!({})).expect("request");

    let ids: Vec<&str> = replies.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, ["a1", "b1", "a2", "a3", "b2"]);
}

/// A grouped script whose `me()` answers with the host's view of its caller.
const VICTIM: &str =
    "import { whoami } from 'host'; export function me(): string { return whoami(); }";

/// Every way a script could name the victim's modules, graph ids 0 to 3: its entry module,
/// its environment module and its instance of the `host` module.
fn victim_module_ids() -> Vec<String> {
    (0..4)
        .flat_map(|graph| {
            [
                format!("manivelle://graph/{graph}/victim"),
                format!("manivelle:env/{graph}"),
                format!("manivelle:host/{graph}/host"),
            ]
        })
        .collect()
}

fn assert_cannot_import_the_victim(
    load: impl Fn(&mut Engine, &str) -> Result<(), manivelle::EngineError>,
) {
    let mut engine = engine();
    engine
        .load_script_in("pack", "victim", VICTIM)
        .expect("load victim");

    for module in victim_module_ids() {
        let attacker = format!(
            "import * as v from '{module}'; export function steal(): unknown {{ return (v as any).me?.(); }}"
        );
        let loaded = load(&mut engine, &attacker);
        let stolen = loaded
            .as_ref()
            .ok()
            .and_then(|()| engine.call::<Option<String>>("attacker", "steal", ()).ok());
        assert!(
            loaded.is_err(),
            "importing `{module}` from another script loaded (steal() = {stolen:?})"
        );
    }
    assert_eq!(
        engine
            .call::<String>("victim", "me", ())
            .expect("victim still works"),
        "victim"
    );
}

#[test]
fn a_standalone_script_cannot_import_another_scripts_modules() {
    assert_cannot_import_the_victim(|engine, source| engine.load_script("attacker", source));
}

#[test]
fn a_script_cannot_import_the_modules_of_another_script_in_its_group() {
    assert_cannot_import_the_victim(|engine, source| {
        engine.load_script_in("pack", "attacker", source)
    });
}

/// `import()` is rejected when a script is compiled, but code a script builds at run
/// time can still call it.
#[test]
fn a_script_cannot_import_another_scripts_modules_at_run_time() {
    let mut engine = engine();
    engine
        .load_script_in("pack", "victim", VICTIM)
        .expect("load victim");
    engine
        .load_script(
            "attacker",
            r#"
            export async function steal(module: string): Promise<unknown> {
              const load = new Function("module", "return imp" + "ort(module)");
              const namespace = await load(module);
              return namespace.me?.();
            }
            "#,
        )
        .expect("load attacker");

    for module in victim_module_ids() {
        let stolen = engine.call::<Option<String>>("attacker", "steal", (module.as_str(),));
        assert!(
            stolen.is_err(),
            "imported `{module}` at run time: {stolen:?}"
        );
    }
}
