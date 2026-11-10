//! A package's permissions are not lifted by loading other code under its id: that load is
//! refused until the package is unloaded, and a reload of the package keeps them.

use std::fs;
use std::path::{Path, PathBuf};

use manivelle::{
    Engine, EngineError, EngineOptions, HostContract, HostContractKind, HostFunctionSignature,
};

struct Modify;

impl HostContract for Modify {
    const NAME: &'static str = "world.modify";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Modify {
    type Input = ();
    type Output = f64;
}

const CALLS: &str = "export const modify = (): number => world.modify();";

/// An engine holding package `pkg`, which lists no permission, and the path of its entry.
fn engine_with_package(root: &Path) -> (Engine, PathBuf) {
    let engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with::<Modify>(|()| Ok(7.0))
        .expect("register");
    let dir = root.join("pkg");
    fs::create_dir_all(&dir).expect("package dir");
    fs::write(
        dir.join("manivelle.kdl"),
        "package \"pkg\" {\n version \"1.0.0\"\n entry \"main.ts\"\n}\n",
    )
    .expect("manifest");
    fs::write(dir.join("main.ts"), CALLS).expect("entry");
    let mut engine = engine;
    let load = engine
        .load_packages(&[root.to_path_buf()])
        .expect("load packages");
    assert!(load.failed.is_empty(), "{:?}", load.failed);
    (engine, dir.join("main.ts"))
}

fn assert_denied(engine: &Engine) {
    let result = engine.call::<f64>("pkg", "modify", ());
    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("permission denied") && details.contains("world.modify")),
        "{result:?}"
    );
}

#[test]
fn unrestricted_code_cannot_replace_a_package_script_in_place() {
    let root = tempfile::tempdir().expect("tempdir");
    let (mut engine, entry) = engine_with_package(root.path());

    let refusals = [
        engine.load_project("pkg", &entry),
        engine.load_project_in("group", "pkg", &entry),
        engine.load_script("pkg", CALLS),
        engine.load_script_in("group", "pkg", CALLS),
    ];

    for refusal in refusals {
        assert!(
            matches!(&refusal, Err(EngineError::ScriptRestricted { script_id }) if script_id == "pkg"),
            "{refusal:?}"
        );
    }
    assert_denied(&engine);
}

#[test]
fn unloading_a_package_script_frees_its_id_for_unrestricted_code() {
    let root = tempfile::tempdir().expect("tempdir");
    let (mut engine, _) = engine_with_package(root.path());

    engine.unload_script("pkg").expect("unload");
    engine.load_script("pkg", CALLS).expect("load unrestricted");

    assert_eq!(engine.call::<f64>("pkg", "modify", ()).expect("call"), 7.0);
}

#[test]
fn reloading_a_changed_package_keeps_its_permissions() {
    let root = tempfile::tempdir().expect("tempdir");
    let (mut engine, entry) = engine_with_package(root.path());

    fs::write(&entry, format!("{CALLS}\nexport const edited = 1;")).expect("edit");
    let report = engine.reload_changed();

    assert_eq!(report.reloaded, ["pkg"], "{:?}", report.failed);
    assert_denied(&engine);
}
