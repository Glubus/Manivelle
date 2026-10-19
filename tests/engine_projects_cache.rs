//! The disk cache of transpiled modules: what a load reuses, what it invalidates, and how
//! cache failures are tolerated.

mod support;

use std::fs;

use manivelle::{Engine, EngineError};
use serde_json::json;
use support::{TestCacheDir, call, fixture};

const DEMO_SCRIPT: &str = include_str!("projects/basic_math/main.ts");

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
