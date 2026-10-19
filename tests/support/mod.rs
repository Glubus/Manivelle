//! Helpers shared by the integration tests. Each test crate compiles this whole module
//! and uses only part of it, so unused items are allowed here rather than per item.
#![allow(dead_code)]

use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Output};
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use manivelle::{
    Engine, EngineError, EngineOptions, HostCallback, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature,
};
use serde_json::{Value, json};

/// An engine with the default options.
pub(crate) fn engine() -> Engine {
    Engine::new(&EngineOptions::default()).expect("create engine")
}

/// Writes `content` to `path`, creating the parent directories first.
pub(crate) fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("file has a parent")).expect("create directory");
    fs::write(path, content).expect("write file");
}

pub(crate) fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

pub(crate) fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

/// Polls `ready` every 20 ms until it returns `true` or `timeout` elapses; returns whether
/// it became ready.
pub(crate) fn wait_until(timeout: Duration, mut ready: impl FnMut() -> bool) -> bool {
    let started = Instant::now();
    loop {
        if ready() {
            return true;
        }
        if started.elapsed() >= timeout {
            return false;
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Writes a config whose `compilerOptions` map `@target` to `target`, resolved from
/// `base_url`.
pub(crate) fn write_paths_config(path: &Path, base_url: &str, target: &str) {
    let config = json!({
        "compilerOptions": { "baseUrl": base_url, "paths": { "@target": [target] } },
    });
    write_file(path, &config.to_string());
}

/// Runs the pinned TypeScript compiler with `flags` and `input`. `None` when TypeScript is
/// not installed locally; in CI (or with `MANIVELLE_REQUIRE_TSC`) that is a failure.
fn run_pinned_tsc(flags: &[&str], input: &OsStr) -> Option<Output> {
    let compiler = Path::new(env!("CARGO_MANIFEST_DIR")).join("node_modules/typescript/bin/tsc");
    let required =
        std::env::var_os("CI").is_some() || std::env::var_os("MANIVELLE_REQUIRE_TSC").is_some();
    if !compiler.is_file() {
        assert!(!required, "TypeScript is required in CI: run npm ci first");
        eprintln!("TypeScript check skipped locally: run npm ci to enable it");
        return None;
    }
    Some(
        Command::new("node")
            .arg(compiler)
            .args(flags)
            .arg(input)
            .output()
            .expect("could not execute the pinned TypeScript compiler with node"),
    )
}

/// Type-checks the single file `path`.
pub(crate) fn run_tsc(path: &Path) -> Option<Output> {
    run_pinned_tsc(
        &["--noEmit", "--target", "ES2020", "--module", "ES2020"],
        path.as_os_str(),
    )
}

/// Type-checks the project whose `tsconfig.json` is in `directory`.
pub(crate) fn run_tsc_project(directory: &Path) -> Option<Output> {
    run_pinned_tsc(&["-p"], directory.join("tsconfig.json").as_os_str())
}

/// Writes `source` as `usage.ts` in a fresh directory and type-checks it.
pub(crate) fn tsc_on_source(name: &str, source: &str) -> Option<Output> {
    let dir = TestCacheDir::new(name);
    let path = dir.path().join("usage.ts");
    write_file(&path, source);
    run_tsc(&path)
}

/// Type-checks `source` when TypeScript is installed: it must pass, or fail with
/// `expected_error`.
pub(crate) fn assert_tsc(name: &str, source: &str, expected_error: Option<&str>) {
    let Some(output) = tsc_on_source(name, source) else {
        return;
    };
    let diagnostics = String::from_utf8_lossy(&output.stdout);
    match expected_error {
        None => assert!(output.status.success(), "tsc on {name}:\n{diagnostics}"),
        Some(code) => assert!(
            !output.status.success() && diagnostics.contains(code),
            "tsc on {name}:\n{diagnostics}"
        ),
    }
}

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct TestCacheDir {
    path: PathBuf,
}

impl TestCacheDir {
    pub(crate) fn new(prefix: &str) -> Self {
        let path = unique_path(prefix);
        fs::create_dir_all(&path).expect("create test cache dir");
        Self { path }
    }

    /// Engine options caching transpiled artifacts in [`TestCacheDir::cache_path`].
    pub(crate) fn engine_options(&self) -> EngineOptions {
        EngineOptions {
            cache_dir: Some(self.cache_path()),
            ..EngineOptions::default()
        }
    }

    /// Transpile cache directory, kept apart from project files written under `path()`.
    pub(crate) fn cache_path(&self) -> PathBuf {
        self.path.join("cache")
    }

    pub(crate) fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for TestCacheDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_path(prefix: &str) -> PathBuf {
    let sequence = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "manivelle-{prefix}-{}-{timestamp}-{sequence}",
        std::process::id()
    ))
}

/// The `math.double` host function of the engine tests: `math.double(n)` is `2 * n`.
pub(crate) struct Double;

impl HostContract for Double {
    const NAME: &'static str = "math.double";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["math", "double"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Double {
    type Input = f64;
    type Output = f64;
}

impl HostFunction for Double {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(input * 2.0)
    }
}

/// An engine with the default options and `source` loaded as the script `script`.
pub(crate) fn engine_with(source: &str) -> Engine {
    let mut engine = engine();
    engine.load_script("script", source).expect("load script");
    engine
}

/// The result of the `count` export of the script `id`.
pub(crate) fn count(engine: &Engine, id: &str) -> f64 {
    engine.call(id, "count", ()).expect("count")
}

/// Writes `root/tsconfig.json`, mapping the alias `@target` to `target`.
pub(crate) fn write_tsconfig(root: &Path, target: &str) {
    write_paths_config(&root.join("tsconfig.json"), ".", target);
}

/// Calls `function` of the script `script` with `args`, panicking with the error if it fails.
pub(crate) fn call(engine: &Engine, script: &str, function: &str, args: Vec<Value>) -> Value {
    engine
        .call::<Value>(script, function, args)
        .unwrap_or_else(|error| panic!("call `{function}` of `{script}`: {error}"))
}

/// The path of `relative` under `tests/projects`.
pub(crate) fn fixture(relative: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("tests/projects")
        .join(relative)
}

/// `import { whoami } from "host"`: the id of the calling script.
pub(crate) struct WhoAmI;

impl HostContract for WhoAmI {
    const NAME: &'static str = "host.whoami";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["whoami"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for WhoAmI {
    type Input = ();
    type Output = String;
}

/// `import { lookup } from "host"`: a score, answered by the host later.
pub(crate) struct Lookup;

impl HostContract for Lookup {
    const NAME: &'static str = "host.lookup";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["lookup"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Lookup {
    type Input = ();
    type Output = f64;
}

/// `import { score } from "host"`; `score.onUpdate(handler)` listens to `score.update`.
pub(crate) struct ScoreUpdate;

impl HostContract for ScoreUpdate {
    const NAME: &'static str = "score.update";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["score", "onUpdate"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for ScoreUpdate {
    type Payload = u32;
}

/// Counts `frame` events, tells who it is, and exposes both.
pub(crate) const COUNTER: &str = r#"
import { ctx } from "manivelle:env";
import { whoami } from "host";

let frames = 0;
ctx.on("frame", () => { frames += 1; });
export function count(): number { return frames; }
export function me(): string { return whoami(); }
"#;

/// An engine whose host answers `host.whoami` with the calling script's id, and that
/// discards console output.
pub(crate) fn group_engine() -> Engine {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with_caller::<WhoAmI>(|caller, ()| Ok(caller.script_id().to_owned()))
        .and_then(|registry| registry.callback::<ScoreUpdate>())
        .expect("register contracts");
    engine.set_console(|_, _, _| {});
    engine
}
