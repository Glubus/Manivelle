//! Delivering an event to one script, and finding out which script an emission failed in.

use std::time::Duration;

use manivelle::{Engine, EngineError, EngineOptions};
use serde_json::json;

const LISTENER: &str = r#"
    let received = 0;
    ctx.on("rank", (n: number) => { received += n; return received; });
    export function read(): number { return received; }
"#;

fn engine_with_listeners(ids: &[&str]) -> Engine {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    for id in ids {
        engine.load_script(*id, LISTENER).expect("load listener");
    }
    engine
}

fn received(engine: &Engine, id: &str) -> f64 {
    engine.call(id, "read", ()).expect("read")
}

#[test]
fn emit_to_reaches_only_the_named_script() {
    let engine = engine_with_listeners(&["a", "b", "c"]);

    let delivered = engine.emit_to("b", "rank", &5).expect("emit_to");

    assert_eq!(delivered, 1);
    assert_eq!(
        [
            received(&engine, "a"),
            received(&engine, "b"),
            received(&engine, "c")
        ],
        [0.0, 5.0, 0.0]
    );
}

#[test]
fn emit_to_a_script_without_a_handler_delivers_nothing() {
    let mut engine = engine_with_listeners(&["a"]);
    engine
        .load_script("quiet", "export function f(): void {}")
        .expect("load");

    assert_eq!(engine.emit_to("quiet", "rank", &1).expect("emit_to"), 0);
    assert_eq!(engine.emit_to("a", "other", &1).expect("emit_to"), 0);
}

#[test]
fn emit_to_an_unloaded_script_is_an_error() {
    let engine = engine_with_listeners(&["a"]);

    let result = engine.emit_to("missing", "rank", &1);

    assert!(
        matches!(result, Err(EngineError::ScriptNotFound { ref script_id }) if script_id == "missing"),
        "{result:?}"
    );
}

#[test]
fn emit_to_returns_the_error_of_the_script_it_targets() {
    let mut engine = engine_with_listeners(&["a"]);
    engine
        .load_script(
            "bad",
            r#"ctx.on("rank", () => { throw new Error("bad mod"); }); export {};"#,
        )
        .expect("load");

    let result = engine.emit_to("bad", "rank", &1);

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("bad mod")),
        "{result:?}"
    );
    assert!(engine.emit_to("a", "rank", &1).is_ok());
}

#[test]
fn request_to_returns_the_replies_of_one_script() {
    let engine = engine_with_listeners(&["a", "b"]);

    let replies: Vec<f64> = engine.request_to("b", "rank", &4).expect("request_to");

    assert_eq!(replies, [4.0]);
    assert_eq!(
        received(&engine, "a"),
        0.0,
        "the other script was not asked"
    );
}

#[test]
fn emit_report_attributes_each_failure_to_its_script() {
    let mut engine = engine_with_listeners(&["ok"]);
    for name in ["first_bad", "second_bad"] {
        engine
            .load_script(
                name,
                &format!(
                    r#"ctx.on("rank", () => {{ throw new Error("{name} threw"); }}); export {{}};"#
                ),
            )
            .expect("load");
    }

    let report = engine.emit_report("rank", &1);

    assert_eq!(report.delivered, 3);
    assert!(report.unattributed.is_none(), "{:?}", report.unattributed);
    let failed: Vec<&str> = report.failures.iter().map(|(id, _)| *id).collect();
    assert_eq!(failed, ["first_bad", "second_bad"]);
    for (id, error) in &report.failures {
        assert!(
            matches!(error, EngineError::Execution { details } if details.contains(&format!("{id} threw"))),
            "{id}: {error:?}"
        );
    }
    assert_eq!(
        received(&engine, "ok"),
        1.0,
        "a failure does not stop the others"
    );
}

#[test]
fn emit_report_names_the_script_that_ran_out_of_time() {
    let mut engine = Engine::new(&EngineOptions {
        execution_timeout: Duration::from_millis(100),
        ..EngineOptions::default()
    })
    .expect("create engine");
    engine.load_script("ok", LISTENER).expect("load");
    engine
        .load_script(
            "slow",
            r#"ctx.on("rank", () => { while (true) {} }); export {};"#,
        )
        .expect("load");

    let report = engine.emit_report("rank", &1);

    let failed: Vec<&str> = report.failures.iter().map(|(id, _)| *id).collect();
    assert_eq!(failed, ["slow"]);
    assert_eq!(received(&engine, "ok"), 1.0);
}

#[test]
fn emit_still_reports_the_first_error_and_a_clean_emission_reports_none() {
    let mut engine = engine_with_listeners(&["ok"]);
    assert!(engine.emit_report("rank", &json!(1)).failures.is_empty());
    engine
        .load_script(
            "bad",
            r#"ctx.on("rank", () => { throw new Error("first"); }); export {};"#,
        )
        .expect("load");

    let result = engine.emit("rank", &json!(1));

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("first")),
        "{result:?}"
    );
}
