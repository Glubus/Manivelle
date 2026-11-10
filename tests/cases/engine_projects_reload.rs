//! Reloading projects: module state, tsconfig changes and failed reloads.

use crate::support;

use std::fs;
use std::path::Path;

use manivelle::{Engine, EngineError};
use serde_json::json;
use support::{
    TestCacheDir, call, engine, fixture, write_file, write_paths_config, write_tsconfig,
};

const ALIAS_ENTRY: &str = r#"
import { label as first } from "./first";
import { label as second } from "./second";
import { label as aliased } from "@target";

export function read() {
  return { first, second, aliased };
}
"#;

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

/// A `paths` change is followed whichever way the project comes back: loaded again by the
/// host (`load_project`), or reloaded because its files changed (`reload_changed`); and
/// whether the `paths` sit in the project's `tsconfig.json` or in a config it `extends`.
#[test]
fn project_reload_follows_a_tsconfig_paths_change() {
    type SetTarget = fn(&Path, &str);
    let plain: SetTarget = |root, target| write_tsconfig(root, target);
    let extended: SetTarget = |root, target| {
        write_paths_config(&root.join("configs").join("base.json"), "..", target);
    };

    for (name, set_target) in [("plain", plain), ("extended", extended)] {
        let cache = TestCacheDir::new(&format!("project-reload-tsconfig-paths-{name}"));
        let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
        let project_root = cache.path().join("project");
        let entry_path = project_root.join("main.ts");
        fs::create_dir_all(&project_root).expect("create project");
        fs::write(&entry_path, ALIAS_ENTRY).expect("write entry");
        for label in ["first", "second"] {
            fs::write(
                project_root.join(format!("{label}.ts")),
                format!("export const label = \"{label}\";\n"),
            )
            .expect("write module");
        }
        if name == "extended" {
            // `tsconfig.json` -> `configs/middle.json` -> `configs/base.json`, which has
            // the `paths`.
            write_file(
                &project_root.join("tsconfig.json"),
                &json!({ "extends": "./configs/middle" }).to_string(),
            );
            write_file(
                &project_root.join("configs").join("middle.json"),
                &json!({ "extends": "./base.json" }).to_string(),
            );
        }

        set_target(&project_root, "./first.ts");
        engine
            .load_project("project", &entry_path)
            .expect("load project");
        let before = call(&engine, "project", "read", Vec::new());
        set_target(&project_root, "./second.ts");
        engine
            .load_project("project", &entry_path)
            .expect("load project again after the paths change");
        let loaded_again = call(&engine, "project", "read", Vec::new());
        set_target(&project_root, "./first.ts");
        let report = engine.reload_changed();
        let reloaded = call(&engine, "project", "read", Vec::new());

        let aliased =
            |label: &str| json!({ "first": "first", "second": "second", "aliased": label });
        assert_eq!(before, aliased("first"), "{name}");
        assert_eq!(loaded_again, aliased("second"), "{name}");
        assert_eq!(report.reloaded, ["project"], "{name}: {report:?}");
        assert_eq!(reloaded, aliased("first"), "{name}");
    }
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
