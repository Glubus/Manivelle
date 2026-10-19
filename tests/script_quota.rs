//! A per-script memory quota inside the engine's shared pool: the script that goes over
//! it fails, and the others keep their share.

use manivelle::{Engine, EngineError, EngineOptions};

const MIB: usize = 1024 * 1024;

/// Holds `n` arrays of about 16 KiB each until `release`.
const HOARDER: &str = r#"
    let held: number[][] = [];
    export function grab(n: number): number {
      for (let i = 0; i < n; i++) held.push(new Array(1024).fill(i));
      return held.length;
    }
    export function release(): void { held = []; }
    ctx.on("grab", (n: number) => grab(n));
"#;

fn engine(script_quota: usize) -> Engine {
    Engine::new(&EngineOptions {
        memory_limit_bytes: 64 * MIB,
        script_memory_limit_bytes: script_quota,
        ..EngineOptions::default()
    })
    .expect("create engine")
}

fn load(engine: &mut Engine, id: &str) {
    engine.load_script(id, HOARDER).expect("load");
}

fn used(engine: &Engine, id: &str) -> u64 {
    engine
        .script_memory_bytes(id)
        .expect("script is loaded")
        .expect("a quota is set")
}

#[test]
fn a_script_over_its_quota_fails_and_its_neighbour_does_not() {
    let mut engine = engine(2 * MIB);
    load(&mut engine, "hog");
    load(&mut engine, "neighbour");

    let hog = engine.call::<f64>("hog", "grab", (500,));

    assert!(
        matches!(hog, Err(EngineError::Execution { ref details })
            if details.contains("`hog`") && details.contains("memory quota")),
        "{hog:?}"
    );
    // 100 arrays are about 1.6 MiB: more than the hog could add, within the quota, and far
    // from filling the 64 MiB pool the hog did not exhaust.
    let neighbour: f64 = engine
        .call("neighbour", "grab", (100,))
        .expect("the neighbour is not the hog's victim");
    assert_eq!(neighbour, 100.0);
}

#[test]
fn a_script_is_charged_for_what_it_holds_and_credited_when_it_lets_go() {
    let mut engine = engine(8 * MIB);
    load(&mut engine, "script");
    load(&mut engine, "other");
    let before = used(&engine, "script");
    let other_before = used(&engine, "other");

    engine.call::<f64>("script", "grab", (100,)).expect("grab");
    let holding = used(&engine, "script");
    engine.call::<()>("script", "release", ()).expect("release");
    engine.run_gc();
    let after = used(&engine, "script");

    assert!(
        holding >= before + 100 * 16 * 1024,
        "holding {holding}, before {before}"
    );
    assert!(
        after < before + 64 * 1024,
        "released: {after}, before {before}"
    );
    assert_eq!(
        used(&engine, "other"),
        other_before,
        "what one script allocates is not charged to another"
    );
}

#[test]
fn faults_over_the_quota_are_attributed_and_quarantine_the_script() {
    let mut engine = engine(MIB);
    load(&mut engine, "hog");
    engine
        .load_script(
            "neighbour",
            r#"ctx.on("grab", (n: number) => n); export {};"#,
        )
        .expect("load");

    for round in 1..=3 {
        let report = engine.emit_report("grab", &500);
        let failed: Vec<&str> = report.failures.iter().map(|(id, _)| id.as_str()).collect();
        assert_eq!(failed, ["hog"], "round {round}");
        let (_, error) = &report.failures[0];
        assert!(
            matches!(error, EngineError::Execution { details }
                if details.contains("memory quota") && details.contains("fault")),
            "{error:?}"
        );
    }

    assert_eq!(engine.quarantined_scripts().collect::<Vec<_>>(), ["hog"]);
    assert!(
        engine.emit_report("grab", &1).failures.is_empty(),
        "a quarantined script is skipped"
    );
}

#[test]
fn without_a_quota_nothing_is_counted() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    load(&mut engine, "script");

    assert_eq!(engine.script_memory_bytes("script").expect("loaded"), None);
    assert!(matches!(
        engine.script_memory_bytes("missing"),
        Err(EngineError::ScriptNotFound { .. })
    ));
}

#[test]
fn a_quota_of_zero_is_refused() {
    let result = Engine::new(&EngineOptions {
        script_memory_limit_bytes: 0,
        ..EngineOptions::default()
    });

    assert!(
        matches!(result, Err(EngineError::InvalidOptions { ref details })
            if details.contains("script_memory_limit_bytes")),
        "{:?}",
        result.err()
    );
}

#[test]
fn reloading_a_script_does_not_leak_its_account() {
    let mut engine = engine(2 * MIB);
    load(&mut engine, "script");
    let baseline = used(&engine, "script");

    for _ in 0..200 {
        load(&mut engine, "script");
        engine.call::<f64>("script", "grab", (20,)).expect("grab");
    }
    engine.call::<()>("script", "release", ()).expect("release");
    engine.run_gc();

    let after = used(&engine, "script");
    assert!(
        after < baseline + 128 * 1024,
        "after 200 reloads {after}, baseline {baseline}"
    );
}

#[test]
fn a_quota_below_what_a_context_needs_fails_the_load_not_the_engine() {
    let mut engine = engine(16 * 1024);

    let loaded = engine.load_script("tiny", HOARDER);

    assert!(loaded.is_err(), "a context does not fit in 16 KiB");
    assert!(
        engine.script_memory_bytes("tiny").is_err(),
        "the failed script is not loaded"
    );
}
