use std::time::Duration;

use manivelle::{Engine, VmError, VmOptions};

#[test]
fn deferred_call_waits_for_a_host_clock_then_returns_its_value() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine
        .load_script(
            "song",
            "export async function beat(): Promise<number> { await new Promise(resolve => setTimeout(resolve, 25)); return 42; }",
        )
        .expect("load");

    let result = engine
        .call_deferred::<f64>("song", "beat", ())
        .expect("call");
    assert!(!result.is_finished());
    engine
        .advance_timers(Duration::from_millis(24))
        .expect("before timer");
    assert!(!result.is_finished());
    engine
        .advance_timers(Duration::from_millis(1))
        .expect("timer fires");
    assert_eq!(result.take().expect("completed").expect("value"), 42.0);
}

#[test]
fn deferred_rejection_is_delivered_only_to_its_handle() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine
        .load_script("song", "export async function beat() { await new Promise(resolve => setTimeout(resolve, 10)); throw new Error('late failure'); }")
        .expect("load");
    let result = engine
        .call_deferred::<f64>("song", "beat", ())
        .expect("call");
    engine
        .advance_timers(Duration::from_millis(10))
        .expect("timer fires");
    let error = result.take().expect("completed").expect_err("rejected");
    assert!(matches!(error, VmError::Execution { details } if details.contains("late failure")));
    assert!(result.take().is_none());
    engine.pump().expect("rejection not reported twice");
}

#[test]
fn deferred_error_stacks_point_to_the_typescript_source() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine
        .load_script(
            "song",
            "export async function fail() { await Promise.resolve(); throw new Error('boom'); }",
        )
        .expect("load");
    let pending = engine
        .call_deferred::<f64>("song", "fail", ())
        .expect("call");
    let error = pending.take().expect("finished").expect_err("rejection");
    assert!(
        matches!(&error, VmError::Execution { details } if details.contains("song.ts:") && details.contains("boom")),
        "{error:?}"
    );
}

#[test]
fn reload_cancels_pending_generation_but_failed_reload_keeps_it() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine.load_script("song", "export async function beat() { await new Promise(resolve => setTimeout(resolve, 10)); return 7; }").expect("load");
    let result = engine
        .call_deferred::<f64>("song", "beat", ())
        .expect("call");
    assert!(engine.load_script("song", "export ???").is_err());
    assert!(!result.is_finished());
    engine
        .load_script("song", "export const next = 1;")
        .expect("reload");
    assert!(matches!(result.take(), Some(Err(VmError::Cancelled))));
    engine
        .advance_timers(Duration::from_millis(10))
        .expect("no stale task");
}

#[test]
fn deferred_request_keeps_delivery_order_when_handlers_resolve_later() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine.load_script("first", "ctx.on('score', async () => { await new Promise(resolve => setTimeout(resolve, 20)); return 'slow'; }); export {};").expect("first");
    engine.load_script("second", "ctx.on('score', async () => { await new Promise(resolve => setTimeout(resolve, 10)); return 'fast'; }); export {};").expect("second");
    let result = engine
        .request_deferred::<(), String>("score", &())
        .expect("request");
    assert!(!result.is_finished());
    engine
        .advance_timers(Duration::from_millis(10))
        .expect("fast");
    assert!(!result.is_finished());
    engine
        .advance_timers(Duration::from_millis(10))
        .expect("slow");
    assert_eq!(
        result.take().expect("ready").expect("replies"),
        vec![
            ("first".to_owned(), "slow".to_owned()),
            ("second".to_owned(), "fast".to_owned()),
        ]
    );
}

#[test]
fn unloading_one_request_participant_cancels_the_whole_request() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine.load_script("one", "ctx.on('score', async () => { await new Promise(resolve => setTimeout(resolve, 10)); return 1; }); export {};").expect("load");
    engine
        .load_script("two", "ctx.on('score', () => 2); export {};")
        .expect("load");
    let request = engine
        .request_deferred::<(), f64>("score", &())
        .expect("request");
    engine.unload_script("one").expect("unload");
    assert!(matches!(request.take(), Some(Err(VmError::Cancelled))));
    engine
        .advance_timers(Duration::from_millis(10))
        .expect("stale timer gone");
}

#[test]
fn retiring_a_completed_participant_cancels_an_incomplete_request() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine
        .load_script("first", "ctx.on('score', () => 1); export {};")
        .expect("load first");
    engine.load_script("second", "ctx.on('score', async () => { await new Promise(resolve => setTimeout(resolve, 10)); return 2; }); export {};").expect("load second");
    let request = engine
        .request_deferred::<(), f64>("score", &())
        .expect("request");
    assert!(!request.is_finished());
    engine
        .unload_script("first")
        .expect("unload replied script");
    assert!(matches!(request.take(), Some(Err(VmError::Cancelled))));
}

#[test]
fn deferred_results_do_not_accumulate_objects_between_collections() {
    let mut engine = Engine::new(&VmOptions::default()).expect("engine");
    engine.load_script("song", "export async function beat() { await new Promise(resolve => setTimeout(resolve, 10)); return { hit: 1 }; }").expect("load");
    let mut baseline = None;
    for count in [10, 100] {
        for _ in 0..count {
            let pending = engine
                .call_deferred::<serde_json::Value>("song", "beat", ())
                .expect("call");
            engine
                .advance_timers(Duration::from_millis(10))
                .expect("timer");
            assert_eq!(
                pending.take().expect("completed").expect("result")["hit"],
                1
            );
        }
        engine.run_gc();
        let objects = engine.memory_stats().object_count;
        if let Some(before) = baseline {
            assert!(
                objects < before + 10,
                "deferred results leaked objects: {before} -> {objects}"
            );
        }
        baseline = Some(objects);
    }
}
