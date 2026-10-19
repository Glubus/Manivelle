//! Scripts of one context group share a QuickJS context but keep their own events, console,
//! timers, host-function identity and callbacks; async host calls settle the promise of the
//! script that made them.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use manivelle::{ConsoleLevel, Engine, EngineOptions, HostResolver};
use serde_json::json;
use support::{COUNTER, Lookup, count, group_engine};

#[test]
fn a_group_shares_one_realm_and_nothing_else_does() {
    let mut engine = group_engine();
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
    let mut engine = group_engine();

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
    let mut engine = group_engine();
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
    let mut engine = group_engine();
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
    let mut engine = group_engine();
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
    let mut engine = group_engine();
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
    let mut engine = group_engine();
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
