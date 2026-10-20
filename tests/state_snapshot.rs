//! `Engine::snapshot` and `Engine::restore` carry the `ctx.hot.save` state of scripts
//! from one engine to another, through text.

use manivelle::{Engine, EngineError, EngineOptions, StateSnapshot};
use serde_json::{Value, json};

const GAME: &str = r#"
    let score = (ctx.hot.data as { score: number } | undefined)?.score ?? 0;
    ctx.hot.save(() => ({ score }));
    export function bump(n: number) { score += n; return score; }
"#;

fn engine() -> Engine {
    Engine::new(&EngineOptions::default()).unwrap()
}

fn snapshot_of(json: &str) -> StateSnapshot {
    serde_json::from_str(json).unwrap()
}

#[test]
fn state_survives_a_round_trip_through_json_text() {
    let mut first = engine();
    first.load_script("game", GAME).unwrap();
    assert_eq!(first.call::<u32>("game", "bump", (5,)).unwrap(), 5);
    assert_eq!(first.call::<u32>("game", "bump", (7,)).unwrap(), 12);

    let text = serde_json::to_string(&first.snapshot().unwrap()).unwrap();
    assert_eq!(text, r#"{"game":{"score":12}}"#);

    let mut second = engine();
    second.restore(snapshot_of(&text)).unwrap();
    second.load_script("game", GAME).unwrap();
    assert_eq!(second.call::<u32>("game", "bump", (1,)).unwrap(), 13);
}

#[test]
fn only_scripts_that_saved_something_are_in_the_snapshot() {
    let mut engine = engine();
    engine.load_script("plain", "export const x = 1;").unwrap();
    engine
        .load_script("nothing", "ctx.hot.save(() => undefined);")
        .unwrap();
    engine
        .load_script("explicit_null", "ctx.hot.save(() => null);")
        .unwrap();
    engine
        .load_script("kept", "ctx.hot.save(() => [1, 2]);")
        .unwrap();

    let snapshot = engine.snapshot().unwrap();

    let ids: Vec<&str> = snapshot.iter().map(|(id, _)| id).collect();
    assert_eq!(ids, ["explicit_null", "kept"], "in id order");
    assert_eq!(snapshot.get("explicit_null"), Some(&Value::Null));
    assert_eq!(snapshot.get("kept"), Some(&json!([1, 2])));
    assert_eq!(snapshot.get("plain"), None);
}

#[test]
fn restoring_a_script_that_is_already_loaded_fails_and_keeps_nothing() {
    let mut engine = engine();
    engine.load_script("a", "export const x = 1;").unwrap();

    let error = engine.restore(snapshot_of(r#"{"a":1,"b":2}"#)).unwrap_err();
    assert!(
        matches!(&error, EngineError::ScriptAlreadyLoaded { script_id } if script_id == "a"),
        "{error}"
    );

    // "b" was in the refused snapshot: it must not have been kept.
    engine
        .load_script(
            "b",
            r#"export function seen() { return ctx.hot.data ?? "none"; }"#,
        )
        .unwrap();
    assert_eq!(engine.call::<String>("b", "seen", ()).unwrap(), "none");
}

#[test]
fn a_restored_state_is_used_once_and_waits_out_a_failed_load() {
    let mut engine = engine();
    engine.restore(snapshot_of(r#"{"g":{"n":3}}"#)).unwrap();
    let reader = r#"export function seen() { return ctx.hot.data ?? "none"; }"#;

    engine
        .load_script("g", "export function (")
        .expect_err("a script that does not parse does not load");
    engine.load_script("g", reader).unwrap();
    assert_eq!(
        engine.call::<Value>("g", "seen", ()).unwrap(),
        json!({"n": 3})
    );

    engine.unload_script("g").unwrap();
    engine.load_script("g", reader).unwrap();
    assert_eq!(
        engine.call::<String>("g", "seen", ()).unwrap(),
        "none",
        "the first successful load used the state up"
    );
}

#[test]
fn a_save_that_fails_names_the_script_and_makes_no_snapshot() {
    for (id, source) in [
        (
            "throws",
            r#"ctx.hot.save(() => { throw new Error("boom"); });"#,
        ),
        ("function", "ctx.hot.save(() => () => 1);"),
        (
            "cycle",
            "ctx.hot.save(() => { const a: unknown[] = []; a.push(a); return a; });",
        ),
        ("map", "ctx.hot.save(() => ({ m: new Map() }));"),
        ("date", "ctx.hot.save(() => new Date());"),
        ("instance", "class P { x = 1 } ctx.hot.save(() => new P());"),
    ] {
        let mut engine = engine();
        engine
            .load_script("good", "ctx.hot.save(() => 1);")
            .unwrap();
        engine.load_script(id, source).unwrap();

        let error = engine.snapshot().unwrap_err().to_string();

        assert!(error.contains(&format!("`{id}`")), "{id}: {error}");
        assert!(error.contains("no snapshot was made"), "{id}: {error}");
    }
}

#[test]
fn the_error_says_where_the_state_stops_being_data() {
    let mut engine = engine();
    engine
        .load_script(
            "game",
            "ctx.hot.save(() => ({ players: [{ name: 'ada', seen: new Set() }] }));",
        )
        .unwrap();

    let error = engine.snapshot().unwrap_err().to_string();

    assert!(error.contains("state.players[0].seen"), "{error}");
    assert!(error.contains("Set"), "{error}");
}

#[test]
fn undefined_inside_plain_data_becomes_null() {
    let mut engine = engine();
    engine
        .load_script(
            "game",
            "ctx.hot.save(() => ({ a: undefined, b: [undefined] }));",
        )
        .unwrap();

    let snapshot = engine.snapshot().unwrap();

    assert_eq!(snapshot.get("game"), Some(&json!({"a": null, "b": [null]})));
}
