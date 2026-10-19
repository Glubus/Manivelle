//! Project module resolution: relative and index imports, ESM forms, type-only references,
//! tsconfig aliases and packages.

mod support;

use manivelle::EngineError;
use serde_json::json;
use support::{call, engine, fixture};

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
