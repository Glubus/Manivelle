//! `VmOptions::builtins`: which optional JavaScript built-ins a script context has.

use manivelle::{Engine, ScriptBuiltins, VmOptions};

/// The switch of one optional built-in.
type Switch = fn(&mut ScriptBuiltins) -> &mut bool;

/// Each optional built-in with the globals it provides.
const OPTIONAL: [(&str, Switch, &[&str]); 6] = [
    ("regexp", |b| &mut b.regexp, &["RegExp"]),
    ("date", |b| &mut b.date, &["Date"]),
    ("proxy", |b| &mut b.proxy, &["Proxy"]),
    (
        "typed_arrays",
        |b| &mut b.typed_arrays,
        &["ArrayBuffer", "Uint8Array", "DataView"],
    ),
    (
        "weak_ref",
        |b| &mut b.weak_ref,
        &["WeakRef", "FinalizationRegistry"],
    ),
    ("web", |b| &mut b.web, &["atob", "btoa", "performance"]),
];

fn engine(builtins: ScriptBuiltins) -> Engine {
    Engine::new(&VmOptions {
        builtins,
        ..VmOptions::default()
    })
    .expect("create engine")
}

/// The names among `OPTIONAL`'s globals that a script of `builtins` can see.
fn visible_globals(builtins: ScriptBuiltins) -> Vec<&'static str> {
    let mut engine = engine(builtins);
    engine
        .load_script(
            "probe",
            r#"
            const names = ["RegExp", "Date", "Proxy", "ArrayBuffer", "Uint8Array", "DataView",
              "WeakRef", "FinalizationRegistry", "atob", "btoa", "performance"];
            export function visible(): string[] {
              return names.filter((name) => typeof (globalThis as any)[name] !== "undefined");
            }
            "#,
        )
        .expect("load script");
    let visible: Vec<String> = engine.call("probe", "visible", ()).expect("call");
    OPTIONAL
        .iter()
        .flat_map(|(_, _, globals)| globals.iter().copied())
        .filter(|name| visible.iter().any(|seen| seen == name))
        .collect()
}

#[test]
fn every_optional_built_in_is_on_by_default() {
    let all: Vec<&str> = OPTIONAL
        .iter()
        .flat_map(|(_, _, globals)| globals.iter().copied())
        .collect();

    assert_eq!(visible_globals(ScriptBuiltins::default()), all);
}

#[test]
fn none_leaves_only_the_core() {
    assert_eq!(visible_globals(ScriptBuiltins::NONE), Vec::<&str>::new());
}

#[test]
fn each_option_adds_exactly_its_own_globals() {
    for (name, field, globals) in OPTIONAL {
        let mut builtins = ScriptBuiltins::NONE;
        *field(&mut builtins) = true;

        assert_eq!(visible_globals(builtins), globals, "only `{name}` enabled");
    }
}

#[test]
fn a_context_with_none_still_runs_what_manivelle_itself_needs() {
    let mut engine = engine(ScriptBuiltins::NONE);
    engine
        .load_script(
            "core",
            r#"
            let ticks = 0;
            setInterval(() => { ticks += 1; }, 0);
            ctx.on("frame", () => { ticks += 10; });
            console.log({ nested: [1, 2] });
            export async function run(): Promise<string> {
              await Promise.resolve();
              const seen = new Map([["a", 1]]);
              return JSON.stringify({ seen: [...seen], ticks });
            }
            "#,
        )
        .expect("load script");

    engine.emit("frame", &0).expect("emit");
    engine
        .advance_timers(std::time::Duration::ZERO)
        .expect("timers");
    let json: String = engine.call("core", "run", ()).expect("call async export");

    assert_eq!(json, r#"{"seen":[["a",1]],"ticks":11}"#);
}

#[test]
fn a_script_using_a_disabled_built_in_fails_where_it_uses_it() {
    let mut engine = engine(ScriptBuiltins {
        date: false,
        ..ScriptBuiltins::ALL
    });
    engine
        .load_script(
            "clock",
            "export function now(): number { return Date.now(); }",
        )
        .expect("loading does not use Date");

    let error = engine
        .call::<f64>("clock", "now", ())
        .expect_err("Date is missing");

    assert!(error.to_string().contains("Date is not defined"), "{error}");
}
