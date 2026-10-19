//! Host functions registered on an engine: imports, globals, JSON calls, errors and contracts.

mod support;

use manivelle::{
    ContractValidation, Engine, EngineError, EngineOptions, HostContract, HostContractKind,
    HostFunction, HostFunctionSignature,
};
use support::{Double, engine};

struct Failing;

impl HostContract for Failing {
    const NAME: &'static str = "host.fail";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Failing {
    type Input = ();
    type Output = f64;
}

impl HostFunction for Failing {
    fn call((): Self::Input) -> Result<Self::Output, EngineError> {
        Err(EngineError::Execution {
            details: String::from("boom"),
        })
    }
}

#[test]
fn typed_host_functions_are_reachable_by_import_and_by_global() {
    let mut engine = engine();
    engine
        .registry()
        .function::<Double>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"
            import { math as imported } from "host";
            export function viaImport(value: number): number { return imported.double(value); }
            export function viaGlobal(value: number): number { return (globalThis as any).math.double(value); }
            "#,
        )
        .expect("load script");

    let via_import: f64 = engine
        .call("script", "viaImport", (21,))
        .expect("call via import");
    let via_global: f64 = engine
        .call("script", "viaGlobal", (4,))
        .expect("call via global");

    assert_eq!(via_import, 42.0);
    assert_eq!(via_global, 8.0);
}

#[test]
fn untyped_host_functions_use_the_json_path() {
    let mut engine = engine();
    engine
        .registry()
        .function::<Double>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"export function run(): number { return (globalThis as any).math.double(2.5); }"#,
        )
        .expect("load script");

    let result: f64 = engine.call("script", "run", ()).expect("call");

    assert_eq!(result, 5.0);
}

#[test]
fn host_function_errors_are_catchable_in_scripts() {
    let mut engine = engine();
    engine
        .registry()
        .function::<Failing>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"
            export function caught(): string {
              try { (globalThis as any).host.fail(); return "no error"; }
              catch (error) { return String((error as Error).message); }
            }
            export function uncaught(): number { return (globalThis as any).host.fail(); }
            "#,
        )
        .expect("load script");

    let caught: String = engine.call("script", "caught", ()).expect("call caught");
    let uncaught = engine.call::<f64>("script", "uncaught", ());

    assert!(caught.contains("boom"), "{caught}");
    assert!(
        matches!(uncaught, Err(EngineError::Execution { ref details }) if details.contains("boom")),
        "{uncaught:?}"
    );
}

#[test]
fn contract_validation_rejects_inputs_outside_the_schema() {
    let mut engine = Engine::new(&EngineOptions {
        contract_validation: ContractValidation::Inputs,
        ..EngineOptions::default()
    })
    .expect("create engine");
    engine
        .registry()
        .function::<Double>()
        .expect("register host function");
    engine
        .load_script(
            "script",
            r#"
            export function valid(): number { return (globalThis as any).math.double(3); }
            export function invalid(): number { return (globalThis as any).math.double("three"); }
            "#,
        )
        .expect("load script");

    let valid: f64 = engine.call("script", "valid", ()).expect("valid input");
    let invalid = engine.call::<f64>("script", "invalid", ());

    assert_eq!(valid, 6.0);
    assert!(
        matches!(invalid, Err(EngineError::Execution { ref details }) if details.contains("validation")),
        "{invalid:?}"
    );
}
