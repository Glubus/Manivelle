//! What a context group hands its scripts: the generated declarations of the environment module,
//! the payload of each delivery, and the load order across groups.

mod support;

use serde_json::json;
use support::{TestCacheDir, group_engine};

#[test]
fn generated_declarations_type_the_environment_module() {
    let dir = TestCacheDir::new("group-env-tsc");
    let engine = group_engine();
    std::fs::write(
        dir.path().join("manivelle.d.ts"),
        engine.registry().dts().expect("render declarations"),
    )
    .expect("write declarations");
    let script = dir.path().join("main.ts");
    std::fs::write(
        &script,
        r#"
        /// <reference path="./manivelle.d.ts" />
        import { ctx, console, setTimeout, setInterval, clearInterval } from "manivelle:env";

        ctx.on("score.update", (combo: number) => { console.log(combo.toFixed(0)); });
        ctx.hot.save(() => ({ at: 1 }));
        const timer = setInterval(() => {}, 10);
        clearInterval(timer);
        setTimeout(() => console.warn("late", 1), 5);
        export {};
        "#,
    )
    .expect("write script");

    let Some(output) = support::run_tsc(&script) else {
        return;
    };

    assert!(
        output.status.success(),
        "tsc rejected a script importing manivelle:env\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn the_scripts_of_a_group_are_handed_one_payload_object_and_other_scripts_their_own() {
    let mut engine = group_engine();
    let mutating = r#"
        import { ctx } from "manivelle:env";
        ctx.on("tick", (event: { n: number }) => { event.n += 1; return event.n; });
        export {};
    "#;
    engine
        .load_script_in("pack", "first", mutating)
        .expect("load");
    engine
        .load_script_in("pack", "second", mutating)
        .expect("load");
    engine
        .load_script_in("elsewhere", "third", mutating)
        .expect("load");
    engine.load_script("fourth", mutating).expect("load");

    let replies: Vec<(&str, u32)> = engine.request("tick", &json!({ "n": 0 })).expect("request");

    assert_eq!(
        replies,
        [("first", 1), ("second", 2), ("third", 1), ("fourth", 1)],
        "a group's scripts share the event they are handed"
    );
}

#[test]
fn a_script_of_another_group_in_between_does_not_break_the_load_order() {
    let mut engine = group_engine();
    let listener = |id: &str| {
        format!(
            r#"
            import {{ ctx }} from "manivelle:env";
            ctx.on("tick", () => "{id}");
            export {{}};
            "#
        )
    };
    for (group, id) in [
        ("a", "a1"),
        ("b", "b1"),
        ("a", "a2"),
        ("a", "a3"),
        ("b", "b2"),
    ] {
        engine
            .load_script_in(group, id, &listener(id))
            .expect("load");
    }

    let replies: Vec<(&str, String)> = engine.request("tick", &json!({})).expect("request");

    let ids: Vec<&str> = replies.iter().map(|(id, _)| *id).collect();
    assert_eq!(ids, ["a1", "b1", "a2", "a3", "b2"]);
}
