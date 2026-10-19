//! What every script context gets besides host contracts: `ctx.off`, `console` and the
//! timer functions on the host-driven clock.

mod support;

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use manivelle::{ConsoleLevel, Engine, EngineError};
use serde_json::json;

use support::engine;

fn engine_with(source: &str) -> Engine {
    let mut engine = engine();
    engine.load_script("script", source).expect("load script");
    engine
}

fn read(engine: &Engine, function: &str) -> f64 {
    engine.call("script", function, ()).expect("read")
}

fn ms(millis: u64) -> Duration {
    Duration::from_millis(millis)
}

// ctx.off

#[test]
fn off_removes_one_registration_of_the_handler() {
    let engine = engine_with(
        r#"let calls = 0;
           const handler = () => { calls += 1; };
           ctx.on("tick", handler);
           ctx.on("tick", handler);
           ctx.off("tick", handler);
           export function read(): number { return calls; }"#,
    );

    engine.emit("tick", &json!(null)).expect("emit");

    assert_eq!(read(&engine, "read"), 1.0);
}

#[test]
fn a_script_whose_last_handler_is_off_no_longer_receives_the_event() {
    let engine = engine_with(
        r#"const handler = () => {};
           ctx.on("tick", handler);
           ctx.off("tick", handler);
           export {};"#,
    );

    let delivered = engine.emit("tick", &json!(null)).expect("emit");

    assert_eq!(delivered, 0);
}

#[test]
fn off_during_a_delivery_still_runs_the_handlers_of_that_delivery() {
    let engine = engine_with(
        r#"const seen: string[] = [];
           const second = () => { seen.push("second"); };
           ctx.on("tick", () => { seen.push("first"); ctx.off("tick", second); });
           ctx.on("tick", second);
           export function read(): string { return seen.join(","); }"#,
    );

    engine.emit("tick", &json!(null)).expect("first emit");
    engine.emit("tick", &json!(null)).expect("second emit");
    let seen: String = engine.call("script", "read", ()).expect("read");

    assert_eq!(seen, "first,second,first");
}

#[test]
fn off_of_a_handler_never_registered_changes_nothing() {
    let engine = engine_with(
        r#"let calls = 0;
           ctx.on("tick", () => { calls += 1; });
           ctx.off("tick", () => {});
           ctx.off("other", () => {});
           export function read(): number { return calls; }"#,
    );

    let delivered = engine.emit("tick", &json!(null)).expect("emit");

    assert_eq!(delivered, 1);
    assert_eq!(read(&engine, "read"), 1.0);
}

#[test]
fn an_event_listened_to_again_after_off_is_delivered() {
    let engine = engine_with(
        r#"let calls = 0;
           const handler = () => { calls += 1; };
           ctx.on("tick", handler);
           ctx.off("tick", handler);
           ctx.on("tick", handler);
           export function read(): number { return calls; }"#,
    );

    let delivered = engine.emit("tick", &json!(null)).expect("emit");

    assert_eq!(delivered, 1);
    assert_eq!(read(&engine, "read"), 1.0);
}

// console

type Lines = Rc<RefCell<Vec<(ConsoleLevel, String, String)>>>;

fn recording(engine: &mut Engine) -> Lines {
    let lines = Lines::default();
    let sink = Rc::clone(&lines);
    engine.set_console(move |level, script_id, message| {
        sink.borrow_mut()
            .push((level, script_id.to_owned(), message.to_owned()));
    });
    lines
}

/// A console sink that panics panics out of the call that logged, whatever the script's
/// `try`/`catch` does.
#[test]
fn a_panicking_console_sink_panics_out_of_the_call_that_logged() {
    let mut engine = engine();
    engine.set_console(|_level, _script, _message| panic!("sink broke"));
    engine
        .load_script(
            "script",
            r#"export function log(): string {
                try { console.log("x"); return "returned"; } catch { return "caught"; }
            }
            export function quiet(): number { return 1; }"#,
        )
        .expect("load script");

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.call::<String>("script", "log", ())
    }));
    let quiet: f64 = engine
        .call("script", "quiet", ())
        .expect("engine stays usable");

    assert!(
        outcome.is_err(),
        "the panic must reach the host: {outcome:?}"
    );
    assert_eq!(quiet, 1.0);
}

#[test]
fn console_calls_reach_the_sink_with_their_level_and_script() {
    let mut engine = engine();
    let lines = recording(&mut engine);

    engine
        .load_script(
            "hud",
            r#"console.log("ready", 3, { combo: 2 }, [1], null, undefined);
               console.debug("d"); console.info("i"); console.warn("w"); console.error("e");
               export {};"#,
        )
        .expect("load script");

    assert_eq!(
        *lines.borrow(),
        [
            (
                ConsoleLevel::Log,
                "hud".to_owned(),
                r#"ready 3 {"combo":2} [1] null undefined"#.to_owned()
            ),
            (ConsoleLevel::Debug, "hud".to_owned(), "d".to_owned()),
            (ConsoleLevel::Info, "hud".to_owned(), "i".to_owned()),
            (ConsoleLevel::Warn, "hud".to_owned(), "w".to_owned()),
            (ConsoleLevel::Error, "hud".to_owned(), "e".to_owned()),
        ]
    );
}

#[test]
fn a_value_without_json_form_is_logged_as_a_string() {
    let mut engine = engine();
    let lines = recording(&mut engine);

    engine
        .load_script(
            "script",
            r#"const cycle: { self?: unknown } = {}; cycle.self = cycle;
               console.log(cycle, 10n, () => 1);
               export {};"#,
        )
        .expect("load script");

    assert_eq!(lines.borrow()[0].2, "[object Object] 10 () => 1");
}

#[test]
fn a_logged_error_shows_its_typescript_location() {
    let mut engine = engine();
    let lines = recording(&mut engine);

    engine
        .load_script(
            "script",
            "export function fail(): void {\n  console.error(new Error(\"boom\"));\n}\n",
        )
        .expect("load script");
    engine.call::<()>("script", "fail", ()).expect("call");

    let message = &lines.borrow()[0].2;
    assert!(message.starts_with("Error: boom\n"), "{message}");
    assert!(message.contains("script.ts:2:"), "{message}");
    assert!(!message.contains("manivelle://"), "{message}");
}

#[test]
fn a_new_sink_also_receives_the_output_of_scripts_already_loaded() {
    let mut engine = engine_with(r#"export function hello(): void { console.log("hello"); }"#);
    let lines = recording(&mut engine);

    engine.call::<()>("script", "hello", ()).expect("call");

    assert_eq!(lines.borrow().len(), 1);
}

// Timers

const TIMER_LOG: &str = r#"
const fired: string[] = [];
export function read(): string { return fired.join(","); }
"#;

fn fired(engine: &Engine) -> String {
    engine.call("script", "read", ()).expect("read")
}

#[test]
fn a_timeout_fires_once_the_clock_reaches_its_delay() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG} setTimeout((name: string) => fired.push(name), 100, "a");"#
    ));

    let before = engine.advance_timers(ms(99)).expect("advance to 99 ms");
    let at_delay = engine.advance_timers(ms(1)).expect("advance to 100 ms");
    let after = engine.advance_timers(ms(500)).expect("advance past");

    assert_eq!((before, at_delay, after), (0, 1, 0));
    assert_eq!(fired(&engine), "a");
}

#[test]
fn due_timers_fire_in_due_order_then_creation_order() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG}
           setTimeout(() => fired.push("late"), 30);
           setTimeout(() => fired.push("early"), 10);
           setTimeout(() => fired.push("tie1"), 20);
           setTimeout(() => fired.push("tie2"), 20);"#
    ));

    engine.advance_timers(ms(50)).expect("advance");

    assert_eq!(fired(&engine), "early,tie1,tie2,late");
}

#[test]
fn an_interval_fires_once_per_call_without_drifting() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG} let n = 0; setInterval(() => fired.push(String(n++)), 10);"#
    ));

    engine.advance_timers(ms(10)).expect("10 ms");
    engine
        .advance_timers(ms(25))
        .expect("35 ms: late by a period, fires once");
    engine
        .advance_timers(ms(0))
        .expect("still 35 ms: the 30 ms period is due");
    engine
        .advance_timers(ms(0))
        .expect("still 35 ms: next period is 40 ms");

    assert_eq!(fired(&engine), "0,1,2");
}

#[test]
fn cleared_timers_never_fire() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG}
           const timeout = setTimeout(() => fired.push("timeout"), 10);
           const interval = setInterval(() => fired.push("interval"), 10);
           setTimeout(() => fired.push("kept"), 10);
           clearTimeout(timeout);
           clearInterval(interval);"#
    ));

    engine.advance_timers(ms(100)).expect("advance");

    assert_eq!(fired(&engine), "kept");
}

#[test]
fn a_timer_cleared_by_an_earlier_callback_of_the_same_call_does_not_fire() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG}
           let second = 0;
           setTimeout(() => {{ fired.push("first"); clearTimeout(second); }}, 10);
           second = setTimeout(() => fired.push("second"), 10);"#
    ));

    engine.advance_timers(ms(10)).expect("advance");

    assert_eq!(fired(&engine), "first");
}

#[test]
fn a_timer_cleared_before_it_is_due_does_not_hide_the_next_one() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG}
           const early = setTimeout(() => fired.push("early"), 10);
           setTimeout(() => fired.push("late"), 50);
           clearTimeout(early);"#
    ));

    let at_early = engine
        .advance_timers(ms(10))
        .expect("advance to the cleared time");
    let at_late = engine
        .advance_timers(ms(40))
        .expect("advance to the late time");

    assert_eq!((at_early, at_late), (0, 1));
    assert_eq!(fired(&engine), "late");
}

#[test]
fn a_timer_set_by_a_script_loaded_later_fires_before_an_earlier_scripts_later_timer() {
    let mut engine = engine_with(&format!(
        r#"{TIMER_LOG}
           setTimeout(() => fired.push("slow"), 100);"#
    ));
    engine
        .load_script(
            "quick",
            &format!(
                r#"{TIMER_LOG}
                   setTimeout(() => fired.push("quick"), 10);"#
            ),
        )
        .expect("load second script");

    let count = engine.advance_timers(ms(10)).expect("advance");

    assert_eq!(count, 1, "only the script with a due timer is entered");
    assert_eq!(fired(&engine), "");
    assert_eq!(
        engine.call::<String>("quick", "read", ()).expect("read"),
        "quick"
    );
}

#[test]
fn a_timer_set_by_a_callback_waits_for_the_next_call() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG}
           setTimeout(() => {{ fired.push("outer"); setTimeout(() => fired.push("inner"), 0); }}, 10);"#
    ));

    engine.advance_timers(ms(10)).expect("first call");
    let after_first = fired(&engine);
    engine.advance_timers(Duration::ZERO).expect("second call");

    assert_eq!(after_first, "outer");
    assert_eq!(fired(&engine), "outer,inner");
}

#[test]
fn a_throwing_timer_does_not_stop_the_others() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG}
           setTimeout(() => {{ throw new Error("timer failed"); }}, 10);
           setTimeout(() => fired.push("after"), 10);"#
    ));

    let result = engine.advance_timers(ms(10));

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("timer failed")),
        "{result:?}"
    );
    assert_eq!(fired(&engine), "after");
}

#[test]
fn an_awaited_timer_resumes_an_event_handler() {
    let engine = engine_with(&format!(
        r#"{TIMER_LOG}
           const sleep = (delay: number) => new Promise<void>(resolve => setTimeout(resolve, delay));
           ctx.on("start", async () => {{ fired.push("start"); await sleep(50); fired.push("end"); }});"#
    ));

    engine.emit("start", &json!(null)).expect("emit");
    let during = fired(&engine);
    engine.advance_timers(ms(50)).expect("advance");

    assert_eq!(during, "start");
    assert_eq!(fired(&engine), "start,end");
}

#[test]
fn timers_share_one_clock_across_scripts_and_skip_scripts_without_due_timers() {
    let mut engine = engine();
    engine
        .load_script("early", "setTimeout(() => {}, 10); export {};")
        .expect("load early");
    engine.advance_timers(ms(10)).expect("advance to 10 ms");
    engine
        .load_script(
            "late",
            &format!("{TIMER_LOG} setTimeout(() => fired.push(\"late\"), 10);"),
        )
        .expect("load late");

    let at_15 = engine.advance_timers(ms(5)).expect("advance to 15 ms");
    let at_20 = engine.advance_timers(ms(5)).expect("advance to 20 ms");
    let late: String = engine.call("late", "read", ()).expect("read");

    assert_eq!((at_15, at_20), (0, 1));
    assert_eq!(late, "late");
}

#[test]
fn a_reload_drops_the_timers_of_the_replaced_version() {
    let mut engine = engine_with(&format!(
        r#"{TIMER_LOG} setTimeout(() => fired.push("old"), 10);"#
    ));

    engine
        .load_script("script", TIMER_LOG)
        .expect("reload without timers");
    let ran = engine.advance_timers(ms(10)).expect("advance");

    assert_eq!(ran, 0);
    assert_eq!(fired(&engine), "");
}

#[test]
fn a_timer_callback_must_be_a_function() {
    let mut engine = engine();

    let result = engine.load_script("script", r#"(setTimeout as any)("code", 10); export {};"#);

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("setTimeout expects a function callback")),
        "{result:?}"
    );
}
