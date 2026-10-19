use std::fs;
use std::path::Path;

mod support;

use manivelle::EngineError;
use support::{engine, write_file};

fn write_mod(root: &Path, name: &str, manifest_body: &str, source: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    write_file(
        &dir.join("manivelle.kdl"),
        &format!(
            "package \"{name}\" {{\n version \"1.0.0\"\n entry \"main.ts\"\n{manifest_body}\n}}\n"
        ),
    );
    write_file(&dir.join("main.ts"), source);
}

#[test]
fn load_packages_loads_dependencies_first_and_isolates_failures() {
    let root = tempfile::tempdir().unwrap();
    let name = |n: &str| format!("export function name(): string {{ return \"{n}\"; }}");
    write_mod(
        root.path(),
        "combat",
        " depends {\n  core version \"^1\"\n }",
        &name("combat"),
    );
    write_mod(root.path(), "core", "", &name("core"));
    write_mod(root.path(), "broken", "", "export function name( {");
    write_mod(
        root.path(),
        "needy",
        " depends {\n  broken\n }",
        &name("needy"),
    );

    let mut engine = engine();
    let load = engine.load_packages(&[root.path().to_path_buf()]).unwrap();

    let loaded: Vec<&str> = load.loaded.iter().map(|m| m.name.as_str()).collect();
    assert_eq!(loaded, ["core", "combat"]);
    let failed: Vec<&str> = load.failed.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(failed, ["broken", "needy"]);
    assert!(
        load.failed[1].reason.contains("dependency `broken`"),
        "{:?}",
        load.failed[1]
    );
    assert_eq!(engine.packages().len(), 2);
    assert_eq!(
        engine.call::<String>("combat", "name", ()).unwrap(),
        "combat"
    );
    assert!(engine.call::<String>("needy", "name", ()).is_err());
}

#[test]
fn a_set_that_cannot_work_together_loads_nothing() {
    let root = tempfile::tempdir().unwrap();
    write_mod(root.path(), "alone", "", "export const x = 1;");
    write_mod(
        root.path(),
        "needy",
        " depends {\n  ghost\n }",
        "export const y = 1;",
    );

    let mut engine = engine();
    let error = engine
        .load_packages(&[root.path().to_path_buf()])
        .unwrap_err();

    assert!(matches!(error, EngineError::Manifest { .. }), "{error}");
    assert!(
        error.to_string().contains("needs package `ghost`"),
        "{error}"
    );
    assert!(engine.packages().is_empty());
    assert!(
        engine.call::<f64>("alone", "x", ()).is_err(),
        "nothing was loaded"
    );
}

#[test]
fn a_directory_without_packages_is_an_error() {
    let empty = tempfile::tempdir().unwrap();
    let error = engine()
        .load_packages(&[empty.path().to_path_buf()])
        .unwrap_err();
    assert!(error.to_string().contains("no manivelle.kdl"), "{error}");
}
