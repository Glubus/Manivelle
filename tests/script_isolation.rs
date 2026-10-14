//! Scripts share nothing but the host: one script's changes to its built-ins must not
//! show up in another's, host functions included.

use manivelle::{
    Engine, HostContract, HostContractKind, HostFunction, HostFunctionSignature, VmError, VmOptions,
};

struct Double;

impl HostContract for Double {
    const NAME: &'static str = "math.double";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Double {
    type Input = f64;
    type Output = f64;
}

impl HostFunction for Double {
    fn call(input: f64) -> Result<f64, VmError> {
        Ok(input * 2.0)
    }
}

fn engine_with_two_scripts() -> Engine {
    let mut engine = Engine::new(&VmOptions::default()).expect("create engine");
    engine
        .registry()
        .function::<Double>()
        .expect("register host function");
    for id in ["first", "second"] {
        engine
            .load_script(
                id,
                r#"
                export function patch(): void {
                  (Function.prototype as any).leaked = "patched by a script";
                }
                export function leaked(): unknown {
                  return (globalThis as any).math.double.leaked;
                }
                export function isOwnFunction(): boolean {
                  return (globalThis as any).math.double instanceof Function;
                }
                "#,
            )
            .expect("load script");
    }
    engine
}

#[test]
fn a_host_function_has_the_function_prototype_of_the_script_that_calls_it() {
    let engine = engine_with_two_scripts();

    for id in ["first", "second"] {
        let own: bool = engine.call(id, "isOwnFunction", ()).expect("call");

        assert!(own, "`math.double instanceof Function` in `{id}`");
    }
}

#[test]
fn patching_function_prototype_in_one_script_does_not_reach_another_scripts_host_functions() {
    let engine = engine_with_two_scripts();
    engine
        .call::<()>("first", "patch", ())
        .expect("patch in the first script");

    let in_first: Option<String> = engine.call("first", "leaked", ()).expect("call");
    let in_second: Option<String> = engine.call("second", "leaked", ()).expect("call");

    assert_eq!(in_first.as_deref(), Some("patched by a script"));
    assert_eq!(in_second, None, "the second script never patched anything");
}
