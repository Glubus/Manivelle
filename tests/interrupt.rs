use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time::{Duration, Instant};

use manivelle::{Engine, InterruptHandle, VmError, VmOptions};

const SCRIPT: &str = r#"
export function spin(): void { while (true) {} }
export function add(a: number, b: number): number { return a + b; }
"#;

/// An engine whose timeout is far longer than the tests, so only the handle can stop it.
fn engine() -> Engine {
    let mut engine = Engine::new(&VmOptions {
        execution_timeout: Duration::from_secs(60),
        ..VmOptions::default()
    })
    .expect("create engine");
    engine.load_script("loops", SCRIPT).expect("load script");
    engine
}

#[test]
fn another_thread_interrupts_a_runaway_call() {
    let engine = engine();
    let handle = engine.interrupt_handle();
    let finished = Arc::new(AtomicBool::new(false));
    // A request sent before the call starts is dropped by design, so keep asking
    // until the call returns.
    let interrupter = thread::spawn({
        let finished = Arc::clone(&finished);
        move || {
            while !finished.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(10));
                handle.interrupt();
            }
        }
    });

    let begin = Instant::now();
    let result = engine.call::<()>("loops", "spin", ());
    finished.store(true, Ordering::Release);
    interrupter.join().expect("interrupter thread");

    assert!(matches!(result, Err(VmError::Interrupted)), "{result:?}");
    assert!(begin.elapsed() < Duration::from_secs(10));
    let sum: f64 = engine
        .call("loops", "add", (2, 3))
        .expect("call after interrupt");
    assert_eq!(sum, 5.0);
}

#[test]
fn an_interrupt_requested_while_idle_does_not_stop_the_next_call() {
    let engine = engine();

    engine.interrupt_handle().interrupt();
    let sum: f64 = engine
        .call("loops", "add", (1, 1))
        .expect("call after idle interrupt");

    assert_eq!(sum, 2.0);
}

#[test]
fn a_timeout_is_not_reported_as_an_interrupt() {
    let mut engine = Engine::new(&VmOptions {
        execution_timeout: Duration::from_millis(50),
        ..VmOptions::default()
    })
    .expect("create engine");
    engine.load_script("loops", SCRIPT).expect("load script");

    let result = engine.call::<()>("loops", "spin", ());

    assert!(
        matches!(result, Err(VmError::Execution { .. })),
        "{result:?}"
    );
}

/// A script whose `tick` handler and timer count their runs, then never return.
const RUNAWAY_LISTENER: &str = r#"
import { ctx, setTimeout } from "manivelle:env";
let ran = 0;
export function count(): number { return ran; }
ctx.on("tick", () => { ran += 1; while (true) {} });
setTimeout(() => { ran += 1; while (true) {} }, 0);
"#;

const LISTENERS: usize = 20;

fn engine_with_listeners(execution_timeout: Duration) -> Engine {
    let mut engine = Engine::new(&VmOptions {
        execution_timeout,
        ..VmOptions::default()
    })
    .expect("create engine");
    for i in 0..LISTENERS {
        engine
            .load_script(format!("s{i}"), RUNAWAY_LISTENER)
            .expect("load script");
    }
    engine
}

/// How many handlers and timers ran, over every script.
fn entered(engine: &Engine) -> f64 {
    (0..LISTENERS)
        .map(|i| {
            engine
                .call::<f64>(&format!("s{i}"), "count", ())
                .expect("count")
        })
        .sum()
}

/// Runs `operation` while another thread keeps asking, through `handle`, to stop.
fn interrupted<T>(handle: InterruptHandle, operation: impl FnOnce() -> T) -> T {
    let finished = Arc::new(AtomicBool::new(false));
    let interrupter = thread::spawn({
        let finished = Arc::clone(&finished);
        move || {
            while !finished.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(10));
                handle.interrupt();
            }
        }
    });
    let result = operation();
    finished.store(true, Ordering::Release);
    interrupter.join().expect("interrupter thread");
    result
}

#[test]
fn an_exhausted_budget_stops_an_emit_at_the_handler_that_used_it_up() {
    let engine = engine_with_listeners(Duration::from_millis(50));

    let result = engine.emit("tick", &1u32);

    assert!(
        matches!(result, Err(VmError::Execution { .. })),
        "{result:?}"
    );
    assert_eq!(
        entered(&engine),
        1.0,
        "no handler runs after the budget is gone"
    );
}

#[test]
fn an_exhausted_budget_stops_a_request_at_the_handler_that_used_it_up() {
    let engine = engine_with_listeners(Duration::from_millis(50));

    let result = engine.request::<_, ()>("tick", &1u32);

    assert!(result.is_err(), "{result:?}");
    assert_eq!(
        entered(&engine),
        1.0,
        "no handler runs after the budget is gone"
    );
}

#[test]
fn an_interrupt_stops_an_emit_at_the_handler_it_reached() {
    let engine = engine_with_listeners(Duration::from_secs(60));

    let result = interrupted(engine.interrupt_handle(), || engine.emit("tick", &1u32));

    assert!(matches!(result, Err(VmError::Interrupted)), "{result:?}");
    assert_eq!(entered(&engine), 1.0, "no handler runs after the interrupt");
}

#[test]
fn an_exhausted_budget_stops_advance_timers_at_the_timer_that_used_it_up() {
    let engine = engine_with_listeners(Duration::from_millis(50));

    let result = engine.advance_timers(Duration::from_millis(1));

    assert!(
        matches!(result, Err(VmError::Execution { .. })),
        "{result:?}"
    );
    assert_eq!(
        entered(&engine),
        1.0,
        "no timer fires after the budget is gone"
    );
}

#[test]
fn an_interrupted_hot_save_is_reported_as_an_interrupt() {
    let mut engine = engine();
    let spinning_save = r#"
    import { ctx } from "manivelle:env";
    ctx.hot.save(() => { while (true) {} });
    export function version(): number { return 1; }
    "#;
    engine.load_script("hot", spinning_save).expect("load v1");

    let handle = engine.interrupt_handle();
    let result = interrupted(handle, || {
        engine.load_script("hot", "export function version(): number { return 2; }")
    });

    assert!(matches!(result, Err(VmError::Interrupted)), "{result:?}");
    let version: f64 = engine.call("hot", "version", ()).expect("call");
    assert_eq!(version, 1.0, "the previous version keeps running");
}

#[test]
fn an_interrupted_hot_dispose_is_reported_as_an_interrupt() {
    let mut engine = engine();
    engine
        .load_script(
            "hot",
            r#"import { ctx } from "manivelle:env"; ctx.hot.dispose(() => { while (true) {} }); export {};"#,
        )
        .expect("load");

    let handle = engine.interrupt_handle();
    let result = interrupted(handle, || engine.unload_script("hot"));

    assert!(matches!(result, Err(VmError::Interrupted)), "{result:?}");
}
