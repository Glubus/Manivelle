//! State kept across hot reloads through `ctx.hot`.

mod support;

use std::cell::RefCell;
use std::path::{Path, PathBuf};

use manivelle::js::{Ctx, Object, Value as JsValue};
use manivelle::{
    Engine, EngineError, HostCallback, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, JsDecode, JsEncode, TsField, TsSchema, TsType,
};

use support::{TestCacheDir, engine, stdout, tsc_on_source, write_file};

const SCRIPT: &str = "script";
const PROJECT: &str = "mod";

/// Keeps a counter across reloads, reading the saved state at its top level.
const COUNTER: &str = r#"
type Saved = { count: number };
let count = (ctx.hot.data as Saved | undefined)?.count ?? 0;
ctx.hot.save(() => ({ count }));
export function next(): number { return ++count; }
"#;

thread_local! {
    static RECORDS: RefCell<Vec<String>> = const { RefCell::new(Vec::new()) };
}

/// `probe.record(text)`: appends `text` to this thread's [`RECORDS`].
struct Record;

impl HostContract for Record {
    const NAME: &'static str = "probe.record";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Record {
    type Input = String;
    type Output = bool;
}

impl HostFunction for Record {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        RECORDS.with_borrow_mut(|records| records.push(input));
        Ok(true)
    }
}

struct Tick;

struct TickPayload {
    frame: f64,
}

impl TsSchema for TickPayload {
    fn schema_name() -> Option<&'static str> {
        Some("TickPayload")
    }

    fn ts_type() -> TsType {
        TsType::Object(vec![TsField::required("frame", TsType::Number)])
    }
}

impl JsEncode for TickPayload {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, manivelle::CodecError> {
        let payload = Object::new(ctx.clone())?;
        payload.set("frame", self.frame)?;
        Ok(payload.into_value())
    }
}

impl HostContract for Tick {
    const NAME: &'static str = "game.tick";

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for Tick {
    type Payload = TickPayload;
}

#[test]
fn a_first_load_sees_no_saved_state() {
    let mut engine = engine();

    engine
        .load_script(
            SCRIPT,
            "export function firstLoad(): boolean { return ctx.hot.data === undefined; }",
        )
        .expect("load script");

    assert!(call::<bool>(&engine, SCRIPT, "firstLoad"));
}

#[test]
fn saved_state_is_read_at_the_top_level_of_the_new_version() {
    let mut engine = engine();
    engine.load_script(SCRIPT, COUNTER).expect("load v1");
    next(&engine);
    next(&engine);

    engine.load_script(SCRIPT, COUNTER).expect("reload");

    assert_eq!(next(&engine), 3.0);
}

#[test]
fn saved_state_is_the_same_object_the_old_version_returned() {
    let mut engine = engine();
    engine
        .load_script(
            SCRIPT,
            r#"
            const shared = { marker: 1 };
            ctx.hot.save(() => shared);
            ctx.hot.dispose(() => { shared.marker = 2; });
            export {};
            "#,
        )
        .expect("load v1");

    engine
        .load_script(
            SCRIPT,
            "export function marker(): number { return (ctx.hot.data as { marker: number }).marker; }",
        )
        .expect("reload");

    // The old version's dispose ran after the swap and mutated the object it handed over.
    assert_eq!(call::<f64>(&engine, SCRIPT, "marker"), 2.0);
}

#[test]
fn the_last_registered_save_wins() {
    let mut engine = engine();
    engine
        .load_script(
            SCRIPT,
            "ctx.hot.save(() => 1); ctx.hot.save(() => 2); export {};",
        )
        .expect("load v1");

    engine
        .load_script(
            SCRIPT,
            "export function data(): unknown { return ctx.hot.data; }",
        )
        .expect("reload");

    assert_eq!(call::<f64>(&engine, SCRIPT, "data"), 2.0);
}

#[test]
fn a_throwing_save_fails_the_reload_and_keeps_the_old_version() {
    let mut engine = probed_engine();
    engine
        .load_script(
            SCRIPT,
            r#"
            let count = 0;
            ctx.hot.save(() => { throw new Error("cannot save"); });
            ctx.hot.dispose(() => probe.record("disposed v1"));
            export function next(): number { return ++count; }
            "#,
        )
        .expect("load v1");
    next(&engine);

    let result = engine.load_script(
        SCRIPT,
        r#"probe.record("v2 loaded"); export function next(): number { return 100; }"#,
    );

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("cannot save") && details.contains("previous version keeps running")),
        "{result:?}"
    );
    assert_eq!(next(&engine), 2.0);
    assert!(take_records().is_empty());
}

#[test]
fn a_new_version_that_fails_to_transpile_keeps_the_old_version_undisposed() {
    let mut engine = probed_engine();
    engine
        .load_script(SCRIPT, &disposing_counter("v1"))
        .expect("load v1");
    next(&engine);

    let result = engine.load_script(SCRIPT, "export function next(: number {");

    assert!(
        matches!(result, Err(EngineError::Transpile { .. })),
        "{result:?}"
    );
    assert_eq!(next(&engine), 2.0);
    assert!(take_records().is_empty());
}

#[test]
fn a_new_version_whose_top_level_throws_keeps_the_old_version_undisposed() {
    let mut engine = probed_engine();
    engine
        .load_script(SCRIPT, &disposing_counter("v1"))
        .expect("load v1");
    next(&engine);

    let result = engine.load_script(
        SCRIPT,
        r#"throw new Error("broken v2"); export function next(): number { return 100; }"#,
    );

    assert!(
        matches!(&result, Err(EngineError::Execution { details }) if details.contains("broken v2")),
        "{result:?}"
    );
    assert_eq!(next(&engine), 2.0);
    assert!(take_records().is_empty());

    engine
        .load_script(SCRIPT, &disposing_counter("v3"))
        .expect("load v3");
    assert_eq!(next(&engine), 3.0);
    assert_eq!(take_records(), ["disposed v1"]);
}

#[test]
fn dispose_callbacks_run_in_order_after_the_new_version_loaded() {
    let mut engine = probed_engine();
    engine
        .load_script(
            SCRIPT,
            r#"
            ctx.hot.dispose(() => probe.record("first"));
            ctx.hot.dispose(() => probe.record("second"));
            export {};
            "#,
        )
        .expect("load v1");

    engine
        .load_script(SCRIPT, r#"probe.record("v2 loaded"); export {};"#)
        .expect("reload");

    assert_eq!(take_records(), ["v2 loaded", "first", "second"]);
}

#[test]
fn unloading_a_script_runs_its_dispose_callbacks() {
    let mut engine = probed_engine();
    engine
        .load_script(SCRIPT, &disposing_counter("v1"))
        .expect("load v1");

    engine.unload_script(SCRIPT).expect("unload");

    assert_eq!(take_records(), ["disposed v1"]);
}

#[test]
fn a_fresh_load_after_unload_sees_no_saved_state() {
    let mut engine = engine();
    engine.load_script(SCRIPT, COUNTER).expect("load v1");
    next(&engine);
    engine.unload_script(SCRIPT).expect("unload");

    engine.load_script(SCRIPT, COUNTER).expect("load again");

    assert_eq!(next(&engine), 1.0);
}

#[test]
fn a_throwing_dispose_fails_the_reload_but_the_new_version_stays_loaded() {
    let mut engine = probed_engine();
    engine
        .load_script(
            SCRIPT,
            r#"
            ctx.hot.dispose(() => { throw new Error("dispose failed"); });
            ctx.hot.dispose(() => probe.record("later dispose"));
            export function version(): number { return 1; }
            "#,
        )
        .expect("load v1");

    let result = engine.load_script(SCRIPT, "export function version(): number { return 2; }");

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("dispose failed") && details.contains("the new version is loaded")),
        "{result:?}"
    );
    assert_eq!(call::<f64>(&engine, SCRIPT, "version"), 2.0);
    assert_eq!(take_records(), ["later dispose"]);
}

#[test]
fn a_throwing_dispose_on_unload_is_returned_and_the_script_is_unloaded() {
    let mut engine = engine();
    engine
        .load_script(
            SCRIPT,
            r#"ctx.hot.dispose(() => { throw new Error("dispose failed"); }); export function run(): void {}"#,
        )
        .expect("load script");

    let result = engine.unload_script(SCRIPT);

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("dispose failed") && details.contains("the script is unloaded")),
        "{result:?}"
    );
    assert!(matches!(
        engine.call::<()>(SCRIPT, "run", ()),
        Err(EngineError::ScriptNotFound { .. })
    ));
}

#[test]
fn reload_changed_hands_saved_state_to_the_edited_project() {
    let dir = TestCacheDir::new("hot-state-project");
    let entry = write_project(dir.path(), "export const step = 1;\n");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project(PROJECT, &entry).expect("load project");
    call::<f64>(&engine, PROJECT, "advance");
    call::<f64>(&engine, PROJECT, "advance");

    write_file(&step_path(dir.path()), "export const step = 100;\n");
    let report = engine.reload_changed();

    assert_eq!(report.reloaded, [PROJECT]);
    assert!(
        report.failed.is_empty() && report.dispose_failed.is_empty(),
        "{report:?}"
    );
    assert_eq!(call::<f64>(&engine, PROJECT, "advance"), 102.0);
}

#[test]
fn reload_changed_reports_a_throwing_dispose_of_a_reloaded_project() {
    let dir = TestCacheDir::new("hot-state-project-dispose");
    let entry = write_project(
        dir.path(),
        r#"ctx.hot.dispose(() => { throw new Error("dispose failed"); }); export const step = 1;"#,
    );
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project(PROJECT, &entry).expect("load project");

    write_file(&step_path(dir.path()), "export const step = 100;\n");
    let report = engine.reload_changed();

    assert_eq!(report.reloaded, [PROJECT]);
    assert!(report.failed.is_empty(), "{report:?}");
    assert!(
        matches!(report.dispose_failed.as_slice(), [(id, EngineError::Execution { details })]
            if id == PROJECT && details.contains("dispose failed")),
        "{report:?}"
    );
    assert_eq!(call::<f64>(&engine, PROJECT, "advance"), 100.0);
}

#[test]
fn generated_declarations_type_ctx_hot() {
    let engine = engine();
    let dts = engine.registry().dts().expect("render declarations");

    assert_typechecks("hot-dts", &format!("{dts}\n{HOT_USAGE}"));
    assert_rejects_misuse("hot-dts-misuse", &format!("{dts}\n{HOT_MISUSE}"));
}

#[test]
fn generated_sdk_types_ctx_hot_next_to_ctx_on() {
    let engine = engine();
    engine
        .registry()
        .callback::<Tick>()
        .expect("register callback");
    let sdk = engine.registry().sdk().expect("render SDK");
    let usage = format!(
        "{sdk}\n{HOT_USAGE}\nctx.on(\"game.tick\", event => {{ event.frame.toFixed(); }});\nmanivelleSdk.ctx.hot.dispose(() => {{}});\n"
    );

    assert_typechecks("hot-sdk", &usage);
    assert_rejects_misuse("hot-sdk-misuse", &format!("{sdk}\n{HOT_MISUSE}"));
}

const HOT_USAGE: &str = r#"
const saved: unknown = ctx.hot.data;
ctx.hot.save(() => ({ saved }));
ctx.hot.dispose(() => {});
"#;

/// Reading a property of `unknown` `data` (TS2339) and saving a non-function (TS2345).
const HOT_MISUSE: &str = r#"
ctx.hot.data.count;
ctx.hot.save(42);
"#;

fn assert_typechecks(name: &str, source: &str) {
    if let Some(output) = tsc_on_source(name, source) {
        assert!(
            output.status.success(),
            "tsc on {name}:\n{}",
            stdout(&output)
        );
    }
}

fn assert_rejects_misuse(name: &str, source: &str) {
    if let Some(output) = tsc_on_source(name, source) {
        let diagnostics = stdout(&output);
        assert!(
            !output.status.success()
                && diagnostics.contains("TS2339")
                && diagnostics.contains("TS2345"),
            "tsc on {name}:\n{diagnostics}"
        );
    }
}

/// An engine with `probe.record`, starting from an empty record list.
fn probed_engine() -> Engine {
    take_records();
    let engine = engine();
    engine
        .registry()
        .function::<Record>()
        .expect("register probe");
    engine
}

fn take_records() -> Vec<String> {
    RECORDS.with_borrow_mut(std::mem::take)
}

/// A counter whose dispose records `disposed {version}`.
fn disposing_counter(version: &str) -> String {
    format!(r#"{COUNTER}ctx.hot.dispose(() => probe.record("disposed {version}"));"#)
}

fn next(engine: &Engine) -> f64 {
    call(engine, SCRIPT, "next")
}

fn call<R: JsDecode>(engine: &Engine, id: &str, function: &str) -> R {
    engine.call(id, function, ()).expect("call export")
}

/// Writes a project whose `advance` adds `step` to a total kept across reloads.
fn write_project(root: &Path, step_module: &str) -> PathBuf {
    let entry = root.join("main.ts");
    write_file(
        &entry,
        r#"
import { step } from "./src/step";
let total = (ctx.hot.data as number | undefined) ?? 0;
ctx.hot.save(() => total);
export function advance(): number { total += step; return total; }
"#,
    );
    write_file(&step_path(root), step_module);
    entry
}

fn step_path(root: &Path) -> PathBuf {
    root.join("src").join("step.ts")
}
