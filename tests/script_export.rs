use manivelle::{
    Engine, EngineOptions, HostContract, HostContractKind, HostFunctionSignature, ScriptExport,
};

/// A host function that happens to have the name of the `add` export.
struct AddContract;

impl HostContract for AddContract {
    const NAME: &'static str = "add";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for AddContract {
    type Input = f64;
    type Output = f64;
}

struct Add;

impl ScriptExport for Add {
    const NAME: &'static str = "add";
    type Args = (f64, f64);
    type Output = f64;
}

struct Reset;

impl ScriptExport for Reset {
    const NAME: &'static str = "reset";
    type Args = ();
    type Output = ();
}

struct Missing;

impl ScriptExport for Missing {
    const NAME: &'static str = "ruun";
    type Args = ();
    type Output = ();
}

fn engine() -> Engine {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .load_script(
            "rules",
            "export function add(a: number, b: number): number { return a + b; }\n\
             export function reset(): void {}",
        )
        .unwrap();
    engine
}

#[test]
fn call_typed_uses_the_declared_name_arguments_and_result() {
    let engine = engine();
    assert_eq!(engine.call_typed::<Add>("rules", (2.0, 3.0)).unwrap(), 5.0);
    engine.call_typed::<Reset>("rules", ()).unwrap();
}

#[test]
fn call_typed_reports_an_export_the_script_lacks() {
    let engine = engine();
    assert!(engine.call_typed::<Missing>("rules", ()).is_err());
}

#[test]
fn registered_exports_become_the_script_exports_type() {
    let engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .script_export::<Reset>()
        .unwrap()
        .script_export::<Add>()
        .unwrap();

    let declarations = engine.registry().dts().unwrap();
    assert!(
        declarations.contains(
            "type ScriptExports = {\n  \"add\": (arg0: number, arg1: number) => number | Promise<number>;\n  \"reset\": () => void | Promise<void>;\n};"
        ),
        "{declarations}"
    );
    let sdk = engine.registry().sdk().unwrap();
    assert!(sdk.contains("export type ScriptExports = {"), "{sdk}");
    // An export is not a host contract: scripts have nothing to call and the contract
    // list the tool shows does not hold it.
    assert!(
        !declarations.contains("declare function add"),
        "{declarations}"
    );
}

#[test]
fn an_export_and_a_host_function_with_the_same_name_coexist() {
    let engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with::<AddContract>(Ok)
        .unwrap()
        .script_export::<Add>()
        .unwrap()
        .script_export::<Add>()
        .unwrap();

    let descriptors = engine.registry().descriptors().unwrap();
    assert_eq!(
        descriptors.len(),
        2,
        "one function, one export (registered twice)"
    );
    let function = engine.registry().get("add").unwrap().unwrap();
    assert_eq!(function.kind, HostContractKind::Function);
}
