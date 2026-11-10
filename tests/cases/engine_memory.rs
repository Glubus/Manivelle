//! The memory limit, and the collection of reference cycles.

use crate::support;

use manivelle::{Engine, EngineError, EngineOptions};
use support::engine_with;

#[test]
fn loading_scripts_past_the_memory_limit_says_the_limit_was_reached() {
    let mut engine = Engine::new(&EngineOptions {
        memory_limit_bytes: 4 * 1024 * 1024,
        ..EngineOptions::default()
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
        matches!(&error, EngineError::Execution { details } if details.contains("out of memory") && details.contains("memory_limit_bytes")),
        "{error:?}"
    );
    let one: f64 = engine
        .call("first", "one", ())
        .expect("engine stays usable");
    assert_eq!(one, 1.0);
}

#[test]
fn exceeding_the_memory_limit_fails_the_call_and_the_engine_stays_usable() {
    let mut engine = Engine::new(&EngineOptions {
        memory_limit_bytes: 8 * 1024 * 1024,
        ..EngineOptions::default()
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

    // QuickJS throws an `InternalError` when it can still build one, and `null` when it ran
    // out of memory while building it: both are the exhaustion, worded either way.
    assert!(
        matches!(&hog, Err(EngineError::Execution { details })
            if details.contains("out of memory") || details.starts_with("non-error exception: Null")),
        "{hog:?}"
    );
    assert_eq!(small, 1.0);
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

/// How many cycles the tests make: two objects each.
const MADE: u32 = 20_000;

#[test]
fn cycles_stay_until_run_gc_when_automatic_collection_is_off() {
    let engine = engine_with(CYCLES);
    engine.set_gc_threshold(None);
    let before = engine.memory_stats().object_count;

    engine
        .call::<()>("script", "makeCycles", (MADE,))
        .expect("make cycles");
    let kept = engine.memory_stats().object_count;
    engine.run_gc();
    let collected = engine.memory_stats().object_count;

    assert!(kept >= before + 2 * u64::from(MADE), "{before} -> {kept}");
    assert!(collected <= before, "{before} -> {collected}");
}

#[test]
fn automatic_collection_resumes_once_a_threshold_is_set_again() {
    let engine = engine_with(CYCLES);
    engine.set_gc_threshold(None);
    engine.set_gc_threshold(Some(64 * 1024));
    let before = engine.memory_stats().object_count;

    engine
        .call::<()>("script", "makeCycles", (MADE,))
        .expect("make cycles");
    let after = engine.memory_stats().object_count;

    // Collected as they were made, so well under the 2 * MADE objects that stay without.
    assert!(after < before + u64::from(MADE), "{before} -> {after}");
}
