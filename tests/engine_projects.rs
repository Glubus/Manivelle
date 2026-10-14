mod support;

use std::fs;
use std::path::Path;

use manivelle::{
    Engine, EngineError, EngineOptions, HostCallback, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, NativeBytes,
};
use serde_json::{Value, json};

use support::TestCacheDir;

const DEMO_SCRIPT: &str = include_str!("projects/basic_math/main.ts");
const HOST_BRIDGE_SCRIPT: &str = include_str!("projects/host_bridge/main.ts");
const LAZY_HOST_BRIDGE_SCRIPT: &str = include_str!("projects/lazy_host_bridge/main.ts");
const THROWING_SCRIPT: &str = include_str!("projects/throwing/main.ts");
const PROJECT_MAIN_V1: &str = include_str!("projects/reloadable_project/main_v1.ts");
const PROJECT_MAIN_V2: &str = include_str!("projects/reloadable_project/main_v2.ts");
const ALIAS_ENTRY: &str = r#"
import { label as first } from "./first";
import { label as second } from "./second";
import { label as aliased } from "@target";

export function read() {
  return { first, second, aliased };
}
"#;

struct FindUser;
struct FindInvoice;
struct ReadNativeBytes;
struct ScoreUpdate;

impl HostContract for FindUser {
    const NAME: &'static str = "user.find";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["user", "find"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindUser {
    type Input = u64;
    type Output = String;
}

impl HostFunction for FindUser {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(format!("user-{input}"))
    }
}

impl HostContract for FindInvoice {
    const NAME: &'static str = "billing.invoice.find";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["billing", "invoice", "find"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindInvoice {
    type Input = u64;
    type Output = String;
}

impl HostFunction for FindInvoice {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(format!("invoice-{input}"))
    }
}

impl HostContract for ReadNativeBytes {
    const NAME: &'static str = "bench.bytes.native";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for ReadNativeBytes {
    type Input = Value;
    type Output = NativeBytes;
}

impl HostFunction for ReadNativeBytes {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        let byte_count = input.get("byteCount").and_then(Value::as_u64).unwrap_or(0) as usize;
        Ok(NativeBytes::new(
            (0..byte_count)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>(),
        ))
    }
}

impl HostContract for ScoreUpdate {
    const NAME: &'static str = "score.update";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["score", "onUpdate"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for ScoreUpdate {
    type Payload = Value;
}

#[test]
fn project_resolves_relative_imports() {
    let mut engine = engine();

    engine
        .load_project("project", fixture("multi_module/main.ts"))
        .expect("load multi-file project");
    let result = call(
        &engine,
        "project",
        "compute",
        vec![json!({ "left": 10, "right": 32 })],
    );

    assert_eq!(result, json!({ "total": 42, "version": "v3" }));
}

#[test]
fn project_resolves_index_modules() {
    let mut engine = engine();

    engine
        .load_project("project", fixture("index_module/main.ts"))
        .expect("load project with index modules");

    assert_eq!(
        call(&engine, "project", "render", vec![json!("world")]),
        json!("hello WORLD")
    );
}

#[test]
fn project_executes_native_esm_import_forms() {
    let mut engine = engine();

    engine
        .load_project("project", fixture("esm_forms/main.ts"))
        .expect("load project with native ESM forms");
    let result = call(
        &engine,
        "project",
        "render",
        vec![json!({ "left": 19, "right": 23 })],
    );

    assert_eq!(result, json!("42 points"));
}

#[test]
fn project_module_state_is_isolated_between_scripts() {
    let mut engine = engine();
    let entry = fixture("stateful_project/main.ts");

    engine
        .load_project("project-a", &entry)
        .expect("load first stateful project");
    engine
        .load_project("project-b", &entry)
        .expect("load second stateful project");
    let first_a = call(&engine, "project-a", "tick", Vec::new());
    let second_a = call(&engine, "project-a", "tick", Vec::new());
    let first_b = call(&engine, "project-b", "tick", Vec::new());

    assert_eq!((first_a, second_a, first_b), (json!(1), json!(2), json!(1)));
}

#[test]
fn project_reload_resets_module_state_for_same_script_id() {
    let mut engine = engine();
    let entry = fixture("stateful_project/main.ts");

    engine
        .load_project("project", &entry)
        .expect("load stateful project");
    let first = call(&engine, "project", "tick", Vec::new());
    let second = call(&engine, "project", "tick", Vec::new());
    engine
        .load_project("project", &entry)
        .expect("reload stateful project");
    let after_reload = call(&engine, "project", "tick", Vec::new());

    assert_eq!(
        (first, second, after_reload),
        (json!(1), json!(2), json!(1))
    );
}

#[test]
fn project_ignores_type_only_module_references() {
    let mut engine = engine();

    engine
        .load_project("project", fixture("type_only_runtime/main.ts"))
        .expect("load project with missing type-only modules");

    assert_eq!(
        call(&engine, "project", "read", vec![json!(null)]),
        json!("empty")
    );
}

#[test]
fn project_resolves_tsconfig_aliases() {
    let mut engine = engine();

    engine
        .load_project("project", fixture("tsconfig_alias/main.ts"))
        .expect("load project with tsconfig aliases");

    assert_eq!(
        call(&engine, "project", "renderInvoice", vec![json!(21)]),
        json!({ "label": "invoice-42", "audit": "feature-ready" })
    );
}

#[test]
fn project_resolves_package_imports_from_local_node_modules() {
    let mut engine = engine();

    engine
        .load_project("project", fixture("package_import/main.ts"))
        .expect("load project with package import");

    assert_eq!(
        call(&engine, "project", "run", vec![json!(4)]),
        json!("demo:12")
    );
}

/// `exports` resolve through the `import` condition, never `require` or `node`, and a
/// package without `exports` through its `module` entry before `main`: the CommonJS
/// files of the fixture would fail to evaluate.
#[test]
fn project_resolves_esm_packages_through_import_conditions_and_module_entries() {
    let mut engine = engine();

    engine
        .load_project("project", fixture("esm_package/main.ts"))
        .expect("load project with ESM packages");

    assert_eq!(
        call(&engine, "project", "run", vec![json!(4)]),
        json!("esm:12:module")
    );
}

#[test]
fn project_rejects_unresolved_package_imports() {
    let result = engine().load_project("project", fixture("invalid_bare_import/main.ts"));

    assert!(
        matches!(&result, Err(EngineError::Resolve { details })
            if details.contains("unable to resolve import `pkg/math`")),
        "{result:?}"
    );
}

#[test]
fn project_rejects_dynamic_imports() {
    let result = engine().load_project("project", fixture("dynamic_import/main.ts"));

    assert!(
        matches!(&result, Err(EngineError::Resolve { details })
            if details.contains("dynamic import() is not supported; use a static import")),
        "{result:?}"
    );
}

/// A rejected script must never reach the cache, or a second load would bypass the check.
#[test]
fn inline_dynamic_imports_stay_rejected_with_a_disk_cache() {
    let cache = TestCacheDir::new("inline-dynamic-import");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    let source = r#"export async function load() { return import("./lazy"); }"#;

    for attempt in 0..2 {
        let result = engine.load_script("inline", source);
        assert!(
            matches!(&result, Err(EngineError::Resolve { details })
                if details.contains("dynamic import() is not supported; use a static import")),
            "attempt {attempt}: {result:?}"
        );
    }
    assert_eq!(cached_artifacts(&cache), 0);
}

#[test]
fn inline_cache_artifact_is_shared_by_scripts_with_the_same_source() {
    let cache = TestCacheDir::new("inline-cache-reuse");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");

    engine
        .load_script("math-a", DEMO_SCRIPT)
        .expect("load first script");
    engine.unload_script("math-a").expect("unload first script");
    engine
        .load_script("math-b", DEMO_SCRIPT)
        .expect("load second script");
    let mut restarted = Engine::new(&cache.engine_options()).expect("restart engine");
    restarted
        .load_script("math-c", DEMO_SCRIPT)
        .expect("load from a restarted engine");

    assert_eq!(cached_artifacts(&cache), 1);
    assert_eq!(
        call(
            &restarted,
            "math-c",
            "sum",
            vec![json!({ "left": 20, "right": 22 })]
        ),
        json!(42)
    );
}

/// A cache only saves work: an artifact that cannot be read, here a directory where
/// the file should be, and that cannot be written over either, must not fail a load.
#[test]
fn an_unreadable_and_unwritable_cache_artifact_does_not_fail_a_load() {
    let cache = TestCacheDir::new("inline-cache-io-error");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    engine
        .load_script("math", DEMO_SCRIPT)
        .expect("load and cache the script");
    let artifacts: Vec<_> = fs::read_dir(cache.cache_path())
        .expect("read cache dir")
        .map(|entry| entry.expect("read cache entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "js"))
        .collect();
    assert_eq!(artifacts.len(), 1);
    for artifact in &artifacts {
        fs::remove_file(artifact).expect("remove artifact");
        fs::create_dir(artifact).expect("put a directory in its place");
    }

    let mut restarted = Engine::new(&cache.engine_options()).expect("restart engine");
    restarted
        .load_script("math", DEMO_SCRIPT)
        .expect("load past an unreadable cache artifact");

    assert_eq!(
        call(
            &restarted,
            "math",
            "sum",
            vec![json!({ "left": 20, "right": 22 })]
        ),
        json!(42)
    );
}

/// The cache holds one artifact per module; loading the same project again, even
/// from a restarted engine, adds none.
#[test]
fn project_cache_artifacts_are_shared_by_loads_of_the_same_project() {
    let cache = TestCacheDir::new("project-cache-reuse");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    let entry = fixture("multi_module/main.ts");

    engine
        .load_project("project-a", &entry)
        .expect("load first project");
    engine
        .unload_script("project-a")
        .expect("unload first project");
    engine
        .load_project("project-b", &entry)
        .expect("load second project");
    let mut restarted = Engine::new(&cache.engine_options()).expect("restart engine");
    restarted
        .load_project("project-c", &entry)
        .expect("load cached project from a restarted engine");

    assert_eq!(cached_artifacts(&cache), 5, "one artifact per module");
    assert_eq!(
        call(
            &restarted,
            "project-c",
            "compute",
            vec![json!({ "left": 1, "right": 2 })]
        ),
        json!({ "total": 3, "version": "v3" })
    );
}

/// Changing one dependency transpiles only that module again.
#[test]
fn project_cache_is_invalidated_when_a_dependency_changes() {
    let cache = TestCacheDir::new("project-cache-invalidation");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    let project_root = cache.path().join("project");
    let src_dir = project_root.join("src");
    let entry_path = project_root.join("main.ts");
    let dependency_path = src_dir.join("value.ts");
    fs::create_dir_all(&src_dir).expect("create project src");
    fs::write(
        &entry_path,
        "import { current } from \"./src/value\";\nexport function read() {\n  return current;\n}\n",
    )
    .expect("write project entry");
    fs::write(&dependency_path, "export const current = 1;\n").expect("write dependency");

    engine
        .load_project("project-a", &entry_path)
        .expect("load first project version");
    let first = call(&engine, "project-a", "read", Vec::new());
    engine
        .unload_script("project-a")
        .expect("unload first project");
    fs::write(&dependency_path, "export const current = 2;\n").expect("rewrite dependency");
    engine
        .load_project("project-b", &entry_path)
        .expect("load second project version");
    let second = call(&engine, "project-b", "read", Vec::new());

    assert_eq!((first, second), (json!(1), json!(2)));
    assert_eq!(cached_artifacts(&cache), 3, "entry, old and new dependency");
}

/// Resolution is recomputed on every load, never read from the cache: a package whose
/// `main` now points to another file resolves to that file.
#[test]
fn project_reload_follows_a_package_manifest_change() {
    let cache = TestCacheDir::new("project-package-manifest-change");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    let project_root = cache.path().join("project");
    let package_dir = project_root.join("node_modules").join("demo-pkg");
    let entry_path = project_root.join("main.ts");
    let manifest_path = package_dir.join("package.json");
    fs::create_dir_all(&package_dir).expect("create package");
    fs::write(
        &entry_path,
        "import { value } from \"demo-pkg\";\nexport function read() {\n  return value;\n}\n",
    )
    .expect("write project entry");
    fs::write(package_dir.join("index.ts"), "export const value = 7;\n")
        .expect("write package entry");
    fs::write(package_dir.join("next.ts"), "export const value = 8;\n")
        .expect("write next package entry");
    fs::write(
        &manifest_path,
        r#"{"name":"demo-pkg","version":"1.0.0","main":"index.ts"}"#,
    )
    .expect("write package manifest");

    engine
        .load_project("project", &entry_path)
        .expect("load first package version");
    let before = call(&engine, "project", "read", Vec::new());
    fs::write(
        &manifest_path,
        r#"{"name":"demo-pkg","version":"1.0.1","main":"next.ts"}"#,
    )
    .expect("rewrite package manifest");
    engine
        .load_project("project", &entry_path)
        .expect("load second package version");
    let after = call(&engine, "project", "read", Vec::new());

    assert_eq!((before, after), (json!(7), json!(8)));
}

#[test]
fn script_calls_a_host_function_through_its_import_namespace() {
    let mut engine = engine();
    engine
        .registry()
        .function::<FindUser>()
        .expect("register host function");

    engine
        .load_script("host-bridge", HOST_BRIDGE_SCRIPT)
        .expect("load host bridge script");

    assert_eq!(
        call(
            &engine,
            "host-bridge",
            "lookupWithNamespace",
            vec![json!(7)]
        ),
        json!("user-7")
    );
}

#[test]
fn host_import_namespace_exposes_only_registered_functions() {
    let mut engine = engine();
    engine
        .registry()
        .function::<FindUser>()
        .and_then(|registry| registry.function::<FindInvoice>())
        .expect("register host functions");

    engine
        .load_script("lazy-host-bridge", LAZY_HOST_BRIDGE_SCRIPT)
        .expect("load lazy host bridge script");

    assert_eq!(
        call(
            &engine,
            "lazy-host-bridge",
            "inspectBindings",
            vec![json!(9)]
        ),
        json!({
            "userFindType": "function",
            "missingType": "undefined",
            "invoice": "invoice-9",
        })
    );
}

#[test]
fn typed_host_function_returns_native_bytes_as_uint8array() {
    let mut engine = engine();
    engine
        .registry()
        .function::<ReadNativeBytes>()
        .expect("register native bytes host function");
    let declarations = engine.registry().dts().expect("render declarations");
    engine
        .load_script(
            "native-bytes",
            r#"
export function inspectNativeBytes(byteCount: number) {
  const bytes = (globalThis as any).__host.callValue("bench.bytes.native", { byteCount });
  return {
    isView: ArrayBuffer.isView(bytes),
    constructorName: bytes.constructor.name,
    length: bytes.length,
    fourth: bytes[3],
    sampled: bytes[0] + bytes[4096],
  };
}
"#,
        )
        .expect("load native bytes script");

    assert!(
        declarations.contains("type NativeBytes = Uint8Array;"),
        "{declarations}"
    );
    assert_eq!(
        call(
            &engine,
            "native-bytes",
            "inspectNativeBytes",
            vec![json!(8192)]
        ),
        json!({
            "isView": true,
            "constructorName": "Uint8Array",
            "length": 8192,
            "fourth": 3,
            "sampled": 80,
        })
    );
}

#[test]
fn script_errors_carry_the_stack_of_the_throwing_function() {
    let mut engine = engine();
    engine
        .load_script("throwing", THROWING_SCRIPT)
        .expect("load throwing script");

    let result = engine.call::<Value>("throwing", "explode", ());

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("boom from script") && details.contains("explode")),
        "{result:?}"
    );
}

#[test]
fn project_reload_replaces_the_dependency_graph_and_its_event_handlers() {
    let cache = TestCacheDir::new("project-reload-graph");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    engine
        .registry()
        .callback::<ScoreUpdate>()
        .expect("register score update callback");
    let project_root = cache.path().join("project");
    let entry_path = project_root.join("main.ts");
    let value_path = project_root.join("value.ts");
    fs::create_dir_all(&project_root).expect("create project");
    fs::write(&entry_path, PROJECT_MAIN_V1).expect("write first entry");
    fs::write(&value_path, "export const current = 1;\n").expect("write first value");

    engine
        .load_project("project", &entry_path)
        .expect("load first project");
    let first_delivered = engine
        .emit("score.update", &json!({ "combo": 5 }))
        .expect("emit first event");
    let first_score = call(&engine, "project", "readScore", Vec::new());

    fs::write(&entry_path, PROJECT_MAIN_V2).expect("write second entry");
    fs::write(&value_path, "export const current = 2;\n").expect("write second value");
    fs::write(project_root.join("extra.ts"), "export const offset = 10;\n")
        .expect("write extra module");
    engine
        .load_project("project", &entry_path)
        .expect("reload project");
    let second_delivered = engine
        .emit("score.update", &json!({ "combo": 5 }))
        .expect("emit second event");
    let second_value = call(&engine, "project", "read", Vec::new());
    let second_score = call(&engine, "project", "readScore", Vec::new());

    assert_eq!((first_delivered, first_score), (1, json!(6)));
    assert_eq!(
        (second_delivered, second_value, second_score),
        (1, json!(12), json!(17))
    );
}

#[test]
fn project_reload_follows_a_tsconfig_paths_change() {
    let cache = TestCacheDir::new("project-reload-tsconfig-paths");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    let project_root = cache.path().join("project");
    let entry_path = project_root.join("main.ts");
    fs::create_dir_all(&project_root).expect("create project");
    fs::write(&entry_path, ALIAS_ENTRY).expect("write entry");
    fs::write(
        project_root.join("first.ts"),
        "export const label = \"first\";\n",
    )
    .expect("write first module");
    fs::write(
        project_root.join("second.ts"),
        "export const label = \"second\";\n",
    )
    .expect("write second module");

    write_alias_tsconfig(&project_root, "./first.ts");
    engine
        .load_project("project", &entry_path)
        .expect("load project");
    let before = call(&engine, "project", "read", Vec::new());
    write_alias_tsconfig(&project_root, "./second.ts");
    engine
        .load_project("project", &entry_path)
        .expect("reload project after tsconfig change");
    let after = call(&engine, "project", "read", Vec::new());

    assert_eq!(
        before,
        json!({ "first": "first", "second": "second", "aliased": "first" })
    );
    assert_eq!(
        after,
        json!({ "first": "first", "second": "second", "aliased": "second" })
    );
}

#[test]
fn realistic_mod_pack_uses_aliases_host_calls_events_and_state() {
    let mut engine = engine();
    engine
        .registry()
        .function::<FindUser>()
        .and_then(|registry| registry.callback::<ScoreUpdate>())
        .expect("register host contracts");

    engine
        .load_project("raid-mod", fixture("realistic_mod_pack/src/main.ts"))
        .expect("load realistic mod pack");
    let damage = call(
        &engine,
        "raid-mod",
        "simulateDamage",
        vec![json!({ "playerId": 7, "baseDamage": 9, "critical": true })],
    );
    let invoice = call(
        &engine,
        "raid-mod",
        "invoiceFor",
        vec![json!(7), json!(12.5)],
    );
    let delivered = engine
        .emit("score.update", &json!({ "combo": 5 }))
        .expect("emit score event");
    let state = call(&engine, "raid-mod", "readState", Vec::new());

    assert_eq!(
        damage,
        json!({ "playerId": 7, "playerName": "user-7", "damage": 18, "score": 18 })
    );
    assert_eq!(invoice, json!("invoice:user-7:12.50"));
    assert_eq!(delivered, 1);
    assert_eq!(
        state,
        json!({
            "score": 23,
            "overlay": {
                "visible": true,
                "messages": ["damage:user-7:18", "combo:5"],
            },
            "session": { "id": "raid-night-01", "ticks": 1 },
        })
    );
}

#[test]
fn failed_reloads_keep_the_previous_script_state_and_handlers() {
    let mut engine = engine();
    engine
        .load_script(
            "script",
            r#"
            let count = 0;
            ctx.on("tick", () => { count += 1; });
            export function next(): number { return ++count; }
            "#,
        )
        .expect("load first version");
    assert_eq!(call(&engine, "script", "next", Vec::new()), json!(1));

    for _ in 0..10 {
        let reload = engine.load_script(
            "script",
            r#"
            ctx.on("tick", () => {});
            throw new Error("broken");
            export function next(): number { return 999; }
            "#,
        );
        assert!(
            matches!(&reload, Err(EngineError::Execution { details }) if details.contains("broken")),
            "{reload:?}"
        );
    }
    let delivered = engine.emit("tick", &json!({})).expect("emit tick");

    assert_eq!(delivered, 1);
    assert_eq!(call(&engine, "script", "next", Vec::new()), json!(3));
}

#[test]
fn failed_project_reload_keeps_the_previous_dependency_graph() {
    let root = TestCacheDir::new("project-rollback");
    let mut engine = Engine::new(&root.engine_options()).expect("create engine");
    let entry = root.path().join("main.ts");
    let dependency = root.path().join("dependency.ts");
    fs::write(
        &entry,
        "import { value } from './dependency'; export function run() { return value; }",
    )
    .expect("write entry");
    fs::write(&dependency, "export const value = 42;").expect("write dependency");
    engine
        .load_project("script", &entry)
        .expect("load first version");

    fs::write(
        &dependency,
        "throw new Error('bad module'); export const value = 99;",
    )
    .expect("break dependency");
    let failed = engine.load_project("script", &entry);
    let kept = call(&engine, "script", "run", Vec::new());
    fs::write(&dependency, "export const value = 7;").expect("fix dependency");
    engine
        .load_project("script", &entry)
        .expect("reload fixed project");

    assert!(
        matches!(&failed, Err(EngineError::Execution { details }) if details.contains("bad module")),
        "{failed:?}"
    );
    assert_eq!(kept, json!(42));
    assert_eq!(call(&engine, "script", "run", Vec::new()), json!(7));
}

fn engine() -> Engine {
    Engine::new(&EngineOptions::default()).expect("create engine")
}

fn call(engine: &Engine, script: &str, function: &str, args: Vec<Value>) -> Value {
    engine
        .call::<Value>(script, function, args)
        .unwrap_or_else(|error| panic!("call `{function}` of `{script}`: {error}"))
}

fn fixture(relative: &str) -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/projects")
        .join(relative)
}

/// Transpiled artifacts in the engine's disk cache, one per distinct cache key.
fn cached_artifacts(cache: &TestCacheDir) -> usize {
    let Ok(entries) = fs::read_dir(cache.cache_path()) else {
        return 0;
    };
    entries
        .map(|entry| entry.expect("read cache entry").path())
        .filter(|path| path.extension().is_some_and(|extension| extension == "js"))
        .count()
}

fn write_alias_tsconfig(project_root: &Path, target: &str) {
    let tsconfig = json!({
        "compilerOptions": {
            "baseUrl": ".",
            "paths": { "@target": [target] },
        },
    });
    fs::write(project_root.join("tsconfig.json"), tsconfig.to_string()).expect("write tsconfig");
}
