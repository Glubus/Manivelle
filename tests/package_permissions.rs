//! A package reaches only the host contracts its `permissions` list; a script loaded
//! without a package is not restricted.

use std::fs;
use std::path::Path;

use manivelle::{
    Engine, EngineError, EngineOptions, HostCallback, HostContract, HostContractKind,
    HostFunctionSignature,
};

struct Add;

impl HostContract for Add {
    const NAME: &'static str = "math.add";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Add {
    type Input = f64;
    type Output = f64;
}

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

struct Ping;

impl HostContract for Ping {
    const NAME: &'static str = "game.ping";

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for Ping {
    type Payload = f64;
}

fn engine() -> Engine {
    let engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with::<Add>(|n| Ok(n + 1.0))
        .unwrap()
        .function_with::<Modify>(|()| Ok(7.0))
        .unwrap()
        .callback::<Ping>()
        .unwrap();
    engine
}

/// A package `name` with `permissions` (the lines of its block, if any) and `source`.
fn write_package(root: &Path, name: &str, permissions: Option<&str>, source: &str) {
    let dir = root.join(name);
    fs::create_dir_all(&dir).unwrap();
    let block = permissions.map_or(String::new(), |lines| {
        format!(" permissions {{\n{lines}\n }}\n")
    });
    fs::write(
        dir.join("manivelle.kdl"),
        format!("package \"{name}\" {{\n version \"1.0.0\"\n entry \"main.ts\"\n{block}}}\n"),
    )
    .unwrap();
    fs::write(dir.join("main.ts"), source).unwrap();
}

const CALLS: &str = "export const add = (): number => math.add(1);\n\
                     export const modify = (): number => world.modify();";

fn denied(result: Result<f64, EngineError>, permission: &str) {
    let Err(EngineError::Execution { details }) = result else {
        panic!("expected a permission failure, got {result:?}");
    };
    assert!(
        details.contains("permission denied") && details.contains(permission),
        "{details}"
    );
}

#[test]
fn a_package_calls_only_the_functions_it_lists() {
    let root = tempfile::tempdir().unwrap();
    write_package(root.path(), "pkg", Some("  math.add"), CALLS);
    let mut engine = engine();

    let load = engine.load_packages(&[root.path().to_path_buf()]).unwrap();
    assert!(load.failed.is_empty(), "{:?}", load.failed);

    assert_eq!(engine.call::<f64>("pkg", "add", ()).unwrap(), 2.0);
    denied(engine.call::<f64>("pkg", "modify", ()), "world.modify");
}

#[test]
fn a_namespace_wildcard_allows_everything_under_it() {
    let root = tempfile::tempdir().unwrap();
    write_package(root.path(), "pkg", Some("  world.*"), CALLS);
    let mut engine = engine();
    engine.load_packages(&[root.path().to_path_buf()]).unwrap();

    assert_eq!(engine.call::<f64>("pkg", "modify", ()).unwrap(), 7.0);
    denied(engine.call::<f64>("pkg", "add", ()), "math.add");
}

#[test]
fn a_package_that_lists_nothing_reaches_nothing() {
    let root = tempfile::tempdir().unwrap();
    write_package(root.path(), "pkg", None, CALLS);
    let mut engine = engine();
    engine.load_packages(&[root.path().to_path_buf()]).unwrap();

    denied(engine.call::<f64>("pkg", "add", ()), "math.add");
    denied(engine.call::<f64>("pkg", "modify", ()), "world.modify");
}

#[test]
fn a_script_loaded_without_a_package_is_not_restricted() {
    let mut engine = engine();
    engine
        .load_script(
            "free",
            "export const go = (): number => world.modify() + math.add(1);",
        )
        .unwrap();

    assert_eq!(engine.call::<f64>("free", "go", ()).unwrap(), 9.0);
}

#[test]
fn listening_to_an_event_needs_its_permission() {
    let root = tempfile::tempdir().unwrap();
    let listen = "let seen = 0;\n\
                  ctx.on(\"game.ping\", (n: number) => { seen += n; });\n\
                  export const total = (): number => seen;";
    write_package(root.path(), "allowed", Some("  game.ping"), listen);
    write_package(root.path(), "refused", Some("  math.add"), listen);
    let mut engine = engine();

    let load = engine.load_packages(&[root.path().to_path_buf()]).unwrap();

    let failed: Vec<&str> = load.failed.iter().map(|f| f.name.as_str()).collect();
    assert_eq!(failed, ["refused"]);
    assert!(
        load.failed[0].reason.contains("permission denied")
            && load.failed[0].reason.contains("game.ping"),
        "{:?}",
        load.failed[0]
    );
    assert_eq!(
        engine.emit("game.ping", &5.0).unwrap(),
        1,
        "only `allowed` listens"
    );
    assert_eq!(engine.call::<f64>("allowed", "total", ()).unwrap(), 5.0);
}

#[test]
fn reloading_a_package_keeps_its_permissions() {
    let root = tempfile::tempdir().unwrap();
    write_package(root.path(), "pkg", Some("  math.add"), CALLS);
    let mut engine = engine();
    engine.load_packages(&[root.path().to_path_buf()]).unwrap();

    fs::write(
        root.path().join("pkg/main.ts"),
        format!("{CALLS}\nexport const version = (): number => 2;"),
    )
    .unwrap();
    assert_eq!(engine.reload_changed().reloaded.len(), 1);

    assert_eq!(engine.call::<f64>("pkg", "version", ()).unwrap(), 2.0);
    denied(engine.call::<f64>("pkg", "modify", ()), "world.modify");
}
