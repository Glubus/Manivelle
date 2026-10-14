mod support;

use std::fs;
use std::path::{Path, PathBuf};

use manivelle::{Engine, VmError};
use serde_json::{Value, json};

use support::TestCacheDir;

/// Entry that keeps a call counter, so a reload (which starts from fresh module
/// state) is observable apart from the value it reads.
const COUNTING_ENTRY: &str = r#"
import { value } from "./src/value";

let calls = 0;

export function read() {
  calls += 1;
  return { value, calls };
}
"#;

#[test]
fn nothing_is_reloaded_while_files_are_unchanged() {
    let dir = TestCacheDir::new("hot-reload-unchanged");
    let entry = write_project(dir.path(), "export const value = 1;\n");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");
    read(&engine, "mod");

    let report = engine.reload_changed();

    assert!(
        report.reloaded.is_empty() && report.failed.is_empty(),
        "{report:?}"
    );
    assert_eq!(read(&engine, "mod"), json!({ "value": 1, "calls": 2 }));
}

#[test]
fn an_edited_dependency_reloads_its_project() {
    let dir = TestCacheDir::new("hot-reload-edit");
    let entry = write_project(dir.path(), "export const value = 1;\n");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");
    read(&engine, "mod");

    write(&value_path(dir.path()), "export const value = 22;\n");
    let report = engine.reload_changed();

    assert_eq!(report.reloaded, ["mod"]);
    assert!(report.failed.is_empty(), "{report:?}");
    assert_eq!(read(&engine, "mod"), json!({ "value": 22, "calls": 1 }));
}

#[test]
fn only_projects_with_changed_files_are_reloaded() {
    let first_dir = TestCacheDir::new("hot-reload-first");
    let second_dir = TestCacheDir::new("hot-reload-second");
    let first = write_project(first_dir.path(), "export const value = 1;\n");
    let second = write_project(second_dir.path(), "export const value = 2;\n");
    let mut engine = Engine::new(&first_dir.engine_options()).expect("create engine");
    engine.load_project("first", &first).expect("load first");
    engine.load_project("second", &second).expect("load second");
    read(&engine, "first");
    read(&engine, "second");

    write(&value_path(second_dir.path()), "export const value = 20;\n");
    let report = engine.reload_changed();

    assert_eq!(report.reloaded, ["second"]);
    assert_eq!(read(&engine, "first"), json!({ "value": 1, "calls": 2 }));
    assert_eq!(read(&engine, "second"), json!({ "value": 20, "calls": 1 }));
}

#[test]
fn a_broken_edit_is_reported_once_and_the_previous_version_keeps_running() {
    let dir = TestCacheDir::new("hot-reload-broken");
    let entry = write_project(dir.path(), "export const value = 1;\n");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");

    write(&value_path(dir.path()), "export const value = ;\n");
    let broken = engine.reload_changed();
    let unchanged = engine.reload_changed();
    let still_running = read(&engine, "mod");
    write(&value_path(dir.path()), "export const value = 333;\n");
    let fixed = engine.reload_changed();

    assert!(broken.reloaded.is_empty());
    assert!(
        matches!(broken.failed.as_slice(), [(id, VmError::Transpile { .. })] if id == "mod"),
        "{broken:?}"
    );
    assert!(
        unchanged.reloaded.is_empty() && unchanged.failed.is_empty(),
        "{unchanged:?}"
    );
    assert_eq!(still_running, json!({ "value": 1, "calls": 1 }));
    assert_eq!(fixed.reloaded, ["mod"]);
    assert_eq!(read(&engine, "mod"), json!({ "value": 333, "calls": 1 }));
}

/// `./src/value` resolved to `src/value/index.ts`; a new `src/value.ts` takes
/// precedence, although no loaded file changed.
#[test]
fn a_new_file_that_changes_an_import_resolution_reloads_the_project() {
    let dir = TestCacheDir::new("hot-reload-new-file");
    let entry = dir.path().join("main.ts");
    write(&entry, COUNTING_ENTRY);
    write(
        &dir.path().join("src").join("value").join("index.ts"),
        "export const value = \"directory\";\n",
    );
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");
    let before = read(&engine, "mod");

    write(&value_path(dir.path()), "export const value = \"file\";\n");
    let report = engine.reload_changed();

    assert_eq!(before["value"], json!("directory"));
    assert_eq!(report.reloaded, ["mod"]);
    assert_eq!(read(&engine, "mod")["value"], json!("file"));
}

#[test]
fn a_tsconfig_paths_change_reloads_the_project() {
    let dir = TestCacheDir::new("hot-reload-tsconfig");
    let entry = dir.path().join("main.ts");
    write(
        &entry,
        "import { label } from \"@target\";\nexport function read() { return label; }\n",
    );
    write(&dir.path().join("a.ts"), "export const label = \"a\";\n");
    write(&dir.path().join("bb.ts"), "export const label = \"bb\";\n");
    write_tsconfig(dir.path(), "./a.ts");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");

    write_tsconfig(dir.path(), "./bb.ts");
    let report = engine.reload_changed();

    assert_eq!(report.reloaded, ["mod"]);
    let label: Value = engine.call("mod", "read", ()).expect("read label");
    assert_eq!(label, json!("bb"));
}

/// The failed reload reached `src/extra.ts`, which the loaded version never imported:
/// fixing only that file must still reload the project.
#[test]
fn fixing_a_module_only_a_failed_reload_imported_reloads_the_project() {
    let dir = TestCacheDir::new("hot-reload-new-broken-module");
    let entry = write_project(dir.path(), "export const value = 1;\n");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");

    let extra = dir.path().join("src").join("extra.ts");
    write(&extra, "export const extra = ;\n");
    write(
        &entry,
        &format!("import \"./src/extra\";\n{COUNTING_ENTRY}"),
    );
    let broken = engine.reload_changed();
    write(&extra, "export const extra = 2;\n");
    let fixed = engine.reload_changed();

    assert!(
        matches!(broken.failed.as_slice(), [(id, _)] if id == "mod"),
        "{broken:?}"
    );
    assert_eq!(fixed.reloaded, ["mod"], "{fixed:?}");
    assert_eq!(read(&engine, "mod"), json!({ "value": 1, "calls": 1 }));
}

/// As above, with a new module that compiles but throws when evaluated.
#[test]
fn fixing_a_module_that_threw_in_a_failed_reload_reloads_the_project() {
    let dir = TestCacheDir::new("hot-reload-new-throwing-module");
    let entry = write_project(dir.path(), "export const value = 1;\n");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");

    let extra = dir.path().join("src").join("extra.ts");
    write(&extra, "throw new Error(\"not ready\");\n");
    write(
        &entry,
        &format!("import \"./src/extra\";\n{COUNTING_ENTRY}"),
    );
    let broken = engine.reload_changed();
    write(&extra, "export const ready = true;\n");
    let fixed = engine.reload_changed();

    assert!(
        matches!(broken.failed.as_slice(), [(id, _)] if id == "mod"),
        "{broken:?}"
    );
    assert_eq!(fixed.reloaded, ["mod"], "{fixed:?}");
}

/// `paths` set in the config `tsconfig.json` extends.
#[test]
fn a_paths_change_in_an_extended_tsconfig_reloads_the_project() {
    let dir = TestCacheDir::new("hot-reload-tsconfig-extends");
    let entry = write_extending_project(dir.path());
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");
    let before: Value = engine.call("mod", "read", ()).expect("read label");

    write_base_tsconfig(dir.path(), "./bb.ts");
    let report = engine.reload_changed();

    assert_eq!(before, json!("a"));
    assert_eq!(report.reloaded, ["mod"], "{report:?}");
    let label: Value = engine.call("mod", "read", ()).expect("read label");
    assert_eq!(label, json!("bb"));
}

#[test]
fn loading_a_project_again_reads_an_edited_extended_tsconfig() {
    let dir = TestCacheDir::new("load-tsconfig-extends");
    let entry = write_extending_project(dir.path());
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine.load_project("mod", &entry).expect("load project");

    write_base_tsconfig(dir.path(), "./bb.ts");
    engine
        .load_project("mod", &entry)
        .expect("load project again");

    let label: Value = engine.call("mod", "read", ()).expect("read label");
    assert_eq!(label, json!("bb"));
}

#[test]
fn inline_scripts_are_never_reloaded() {
    let dir = TestCacheDir::new("hot-reload-inline");
    let mut engine = Engine::new(&dir.engine_options()).expect("create engine");
    engine
        .load_script("inline", "export function read() { return 1; }")
        .expect("load inline script");

    let report = engine.reload_changed();

    assert!(
        report.reloaded.is_empty() && report.failed.is_empty(),
        "{report:?}"
    );
}

/// Writes `main.ts` and `src/value.ts` under `root` and returns the entry path.
fn write_project(root: &Path, value_module: &str) -> PathBuf {
    let entry = root.join("main.ts");
    write(&entry, COUNTING_ENTRY);
    write(&value_path(root), value_module);
    entry
}

fn value_path(root: &Path) -> PathBuf {
    root.join("src").join("value.ts")
}

fn write_tsconfig(root: &Path, target: &str) {
    let tsconfig = json!({
        "compilerOptions": { "baseUrl": ".", "paths": { "@target": [target] } },
    });
    write(&root.join("tsconfig.json"), &tsconfig.to_string());
}

/// A project whose `tsconfig.json` takes `@target` from `configs/base.json`, through
/// two levels of `extends`; returns the entry path.
fn write_extending_project(root: &Path) -> PathBuf {
    let entry = root.join("main.ts");
    write(
        &entry,
        "import { label } from \"@target\";\nexport function read() { return label; }\n",
    );
    write(&root.join("a.ts"), "export const label = \"a\";\n");
    write(&root.join("bb.ts"), "export const label = \"bb\";\n");
    write(
        &root.join("tsconfig.json"),
        &json!({ "extends": "./configs/middle" }).to_string(),
    );
    write(
        &root.join("configs").join("middle.json"),
        &json!({ "extends": "./base.json" }).to_string(),
    );
    write_base_tsconfig(root, "./a.ts");
    entry
}

fn write_base_tsconfig(root: &Path, target: &str) {
    let tsconfig = json!({
        "compilerOptions": { "baseUrl": "..", "paths": { "@target": [target] } },
    });
    write(
        &root.join("configs").join("base.json"),
        &tsconfig.to_string(),
    );
}

fn write(path: &Path, contents: &str) {
    fs::create_dir_all(path.parent().expect("file has a parent")).expect("create directory");
    fs::write(path, contents).expect("write file");
}

fn read(engine: &Engine, id: &str) -> Value {
    engine.call(id, "read", ()).expect("call read")
}
