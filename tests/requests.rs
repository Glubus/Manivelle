//! `Engine::request`: events whose handlers reply, and their generated types.

mod support;

use manivelle::{Engine, EngineError, HostCallback, HostContract, HostContractKind, HostRequest};
use serde_json::json;

use support::{assert_tsc, engine};

/// `menu.label`: asks every script for a label for the item it is given.
struct Label;

impl HostContract for Label {
    const NAME: &'static str = "menu.label";

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for Label {
    type Payload = String;
}

impl HostRequest for Label {
    type Reply = String;
}

fn engine_with(scripts: &[(&str, &str)]) -> Engine {
    let mut engine = engine();
    for (id, source) in scripts {
        engine.load_script(*id, source).expect("load script");
    }
    engine
}

#[test]
fn replies_come_back_in_load_then_registration_order_with_their_script() {
    let engine = engine_with(&[
        (
            "zeta",
            r#"ctx.on("menu.label", item => `zeta:${item}`);
               ctx.on("menu.label", item => `zeta2:${item}`); export {};"#,
        ),
        ("silent", r#"ctx.on("other", () => {}); export {};"#),
        (
            "alpha",
            r#"ctx.on("menu.label", item => `alpha:${item}`); export {};"#,
        ),
    ]);

    let replies: Vec<(&str, String)> = engine.request("menu.label", "save").expect("request");

    assert_eq!(
        replies,
        [
            ("zeta", "zeta:save".to_owned()),
            ("zeta", "zeta2:save".to_owned()),
            ("alpha", "alpha:save".to_owned()),
        ]
    );
}

/// Which scripts answer `menu.label`, in order.
fn answering(engine: &Engine) -> Vec<String> {
    let replies: Vec<(&str, String)> = engine.request("menu.label", "x").expect("request");
    replies.iter().map(|(id, _)| (*id).to_owned()).collect()
}

#[test]
fn the_scripts_that_answer_follow_loads_unloads_and_handler_changes() {
    let mut engine = engine_with(&[
        ("one", r#"ctx.on("menu.label", () => "one"); export {};"#),
        (
            "joiner",
            r#"const handler = () => "joiner";
               export function join(): void { ctx.on("menu.label", handler); }
               export function leave(): void { ctx.off("menu.label", handler); }"#,
        ),
    ]);
    assert_eq!(answering(&engine), ["one"]);

    engine.call::<()>("joiner", "join", ()).expect("join");
    assert_eq!(answering(&engine), ["one", "joiner"]);

    engine.unload_script("one").expect("unload");
    assert_eq!(
        answering(&engine),
        ["joiner"],
        "positions shifted by the unload"
    );

    engine
        .load_script("late", r#"ctx.on("menu.label", () => "late"); export {};"#)
        .expect("load late");
    assert_eq!(answering(&engine), ["joiner", "late"]);

    engine.call::<()>("joiner", "leave", ()).expect("leave");
    assert_eq!(answering(&engine), ["late"]);

    engine
        .load_script("late", "export {};")
        .expect("reload without a handler");
    assert!(answering(&engine).is_empty());
}

#[test]
fn an_async_handler_replies_with_what_its_promise_resolves_to() {
    let engine = engine_with(&[(
        "script",
        r#"ctx.on("menu.label", async item => { await Promise.resolve(); return item.toUpperCase(); });
           export {};"#,
    )]);

    let replies: Vec<(&str, String)> = engine.request("menu.label", "load").expect("request");

    assert_eq!(replies, [("script", "LOAD".to_owned())]);
}

#[test]
fn a_request_nobody_handles_has_no_replies() {
    let engine = engine_with(&[("script", "export {};")]);

    let replies: Vec<(&str, String)> = engine.request("menu.label", "x").expect("request");

    assert!(replies.is_empty());
}

#[test]
fn a_failing_handler_fails_the_request_after_every_handler_ran() {
    let engine = engine_with(&[
        (
            "thrower",
            r#"ctx.on("menu.label", () => { throw new Error("no label"); }); export {};"#,
        ),
        (
            "recorder",
            r#"let asked = 0;
               ctx.on("menu.label", () => { asked += 1; return "ok"; });
               export function read(): number { return asked; }"#,
        ),
    ]);

    let result = engine.request::<_, String>("menu.label", "x");
    let asked: f64 = engine.call("recorder", "read", ()).expect("read");

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("no label")),
        "{result:?}"
    );
    assert_eq!(asked, 1.0);
}

#[test]
fn a_rejected_async_handler_fails_the_request_once() {
    let engine = engine_with(&[(
        "script",
        r#"ctx.on("menu.label", async () => { throw new Error("async no label"); }); export {};"#,
    )]);

    let result = engine.request::<_, String>("menu.label", "x");
    let next = engine.emit("unrelated", &json!(null));

    assert!(
        matches!(result, Err(EngineError::Execution { ref details }) if details.contains("async no label")),
        "{result:?}"
    );
    assert!(
        next.is_ok(),
        "the rejection must not leak as unhandled: {next:?}"
    );
}

#[test]
fn a_reply_of_the_wrong_type_fails_the_request() {
    let engine = engine_with(&[("script", r#"ctx.on("menu.label", () => 42); export {};"#)]);

    let result = engine.request::<_, String>("menu.label", "x");

    assert!(
        matches!(result, Err(EngineError::Execution { .. })),
        "{result:?}"
    );
}

#[test]
fn a_handler_waiting_on_the_host_fails_the_request() {
    let engine = engine_with(&[(
        "script",
        r#"ctx.on("menu.label", () => new Promise(() => {})); export {};"#,
    )]);

    let result = engine.request::<_, String>("menu.label", "x");

    assert!(
        matches!(result, Err(EngineError::Execution { ref details })
            if details.contains("a `menu.label` handler of script `script`") && details.contains("never settles")),
        "{result:?}"
    );
}

#[test]
fn generated_declarations_type_request_replies() {
    let engine = engine();
    engine
        .registry()
        .request::<Label>()
        .expect("register request");
    let dts = engine.registry().dts().expect("render declarations");

    assert_tsc(
        "request-dts",
        &format!(
            "{dts}\nctx.on(\"menu.label\", item => item.toUpperCase());\nctx.on(\"menu.label\", async item => item);\n"
        ),
        None,
    );
    // A number is not the declared string reply (TS2322).
    assert_tsc(
        "request-dts-misuse",
        &format!("{dts}\nctx.on(\"menu.label\", () => 42);\n"),
        Some("TS2322"),
    );
}

#[test]
fn generated_sdk_types_request_replies_and_off() {
    let engine = engine();
    engine
        .registry()
        .request::<Label>()
        .expect("register request");
    let sdk = engine.registry().sdk().expect("render SDK");

    assert_tsc(
        "request-sdk",
        &format!(
            "{sdk}\nconst label = (item: string) => `${{item}}!`;\nctx.on(\"menu.label\", label);\nctx.off(\"menu.label\", label);\n"
        ),
        None,
    );
    assert_tsc(
        "request-sdk-misuse",
        &format!("{sdk}\nctx.on(\"menu.label\", () => 42);\n"),
        Some("TS2322"),
    );
}
