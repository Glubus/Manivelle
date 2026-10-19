//! The program `manivelle check --from` starts in the `cli_check` tests: an application that
//! registers the `math.add` contract with `f64` values and opts in by calling
//! `Engine::setup_protocol`. It is an example so that the tests can run it as a separate
//! process, and `cargo test` builds it next to the test binaries.

use manivelle::{Engine, EngineOptions, HostContract, HostContractKind, HostFunctionSignature};

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

fn main() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with::<Add>(Ok)
        .expect("register");
    engine.setup_protocol().expect("set up the protocol");
}
