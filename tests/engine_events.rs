//! Event delivery to the handlers scripts register with `ctx.on`.

mod support;

use manivelle::{Engine, EngineError, EngineOptions};
use serde_json::json;
use support::{engine, engine_with};

/// A delivery in progress runs the handler list it took: a handler removed meanwhile
/// still runs in it, one added meanwhile waits for the next delivery.
#[test]
fn a_delivery_runs_the_snapshot_it_took_even_when_handlers_change_the_list() {
    let engine = engine_with(
        r#"
        const order: string[] = [];
        const late = () => { order.push("late"); };
        const second = (event: { n: number }) => { order.push(`second:${event.n}`); };
        const first = (event: { n: number }) => {
            order.push(`first:${event.n}`);
            ctx.off("tick", second);
            ctx.on("tick", late);
        };
        ctx.on("tick", first);
        ctx.on("tick", second);
        export function read(): string { return order.join(","); }
        export function clear(): void { order.length = 0; }
        "#,
    );

    engine
        .emit("tick", &json!({ "n": 7 }))
        .expect("first delivery");
    let first: String = engine.call("script", "read", ()).expect("read");
    engine.call::<()>("script", "clear", ()).expect("clear");
    engine
        .emit("tick", &json!({ "n": 8 }))
        .expect("second delivery");
    let second: String = engine.call("script", "read", ()).expect("read");

    assert_eq!(
        first, "first:7,second:7",
        "`second` was still in the list taken"
    );
    assert_eq!(
        second, "first:8,late",
        "the next delivery sees the new list"
    );
}

#[test]
fn a_throwing_event_handler_fails_the_emit() {
    let engine =
        engine_with(r#"ctx.on("tick", () => { throw new Error("handler failed"); }); export {};"#);

    let result = engine.emit("tick", &json!({}));

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("handler failed")),
        "{result:?}"
    );
}

#[test]
fn emit_calls_every_registered_handler() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .load_script(
            "listener",
            r#"
            let total = 0;
            ctx.on("tick", event => { total += event.n; });
            ctx.on("tick", event => { total += event.n * 10; });
            export function read(): number { return total; }
            "#,
        )
        .expect("load script");

    let delivered = engine.emit("tick", &json!({ "n": 2 })).expect("emit");
    let total: f64 = engine.call("listener", "read", ()).expect("read total");

    assert_eq!(delivered, 1);
    assert_eq!(total, 22.0);
}

#[test]
fn a_handler_registered_by_a_later_call_receives_events() {
    let engine = engine_with(
        r#"
        let received = 0;
        export function subscribe(): void { ctx.on("tick", () => { received += 1; }); }
        export function read(): number { return received; }
        "#,
    );

    let before = engine
        .emit("tick", &json!(null))
        .expect("emit before subscribing");
    engine
        .call::<()>("script", "subscribe", ())
        .expect("subscribe");
    let after = engine
        .emit("tick", &json!(null))
        .expect("emit after subscribing");
    let received: f64 = engine.call("script", "read", ()).expect("read count");

    assert_eq!((before, after, received), (0, 1, 1.0));
}

#[test]
fn a_reload_without_handlers_stops_receiving_events() {
    let mut engine = engine_with(r#"ctx.on("tick", () => {}); export {};"#);
    engine
        .load_script("script", "export function idle(): void {}")
        .expect("reload without handlers");

    let delivered = engine.emit("tick", &json!(null)).expect("emit");

    assert_eq!(delivered, 0);
}

#[test]
fn a_throwing_handler_does_not_stop_delivery_to_other_handlers() {
    let mut engine = engine();
    let recorder = r#"
        let received = 0;
        ctx.on("tick", () => { received += 1; });
        export function read(): number { return received; }
    "#;
    engine
        .load_script(
            "thrower",
            &format!(
                r#"ctx.on("tick", () => {{ throw new Error("handler failed"); }});{recorder}"#
            ),
        )
        .expect("load thrower");
    engine
        .load_script("recorder", recorder)
        .expect("load recorder");

    let result = engine.emit("tick", &json!(null));
    let thrower_received: f64 = engine.call("thrower", "read", ()).expect("read thrower");
    let recorder_received: f64 = engine.call("recorder", "read", ()).expect("read recorder");

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("handler failed")),
        "{result:?}"
    );
    assert_eq!(thrower_received, 1.0);
    assert_eq!(recorder_received, 1.0);
}

#[test]
fn emit_reports_the_first_failure_in_load_order() {
    let mut engine = engine();
    for name in ["zeta", "alpha", "mid", "beta", "omega", "gamma"] {
        engine
            .load_script(
                name,
                &format!(
                    r#"ctx.on("tick", () => {{ throw new Error("from {name}"); }}); export {{}};"#
                ),
            )
            .expect("load script");
    }
    engine
        .load_script(
            "zeta",
            r#"ctx.on("tick", () => { throw new Error("from zeta v2"); }); export {};"#,
        )
        .expect("reload keeps load position");

    let result = engine.emit("tick", &json!(null));

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("from zeta v2")),
        "{result:?}"
    );
}
