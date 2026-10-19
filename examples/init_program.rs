//! The program `manivelle init` starts in the `cli_init` tests: an application that opted in
//! by calling `Engine::setup_protocol`. It is an example rather than a test so that the tests
//! can run it as a separate process, and `cargo test` builds it next to the test binaries.
//!
//! `MANIVELLE_TEST_HANG` makes it sleep before opting in, and the argument `no-opt-in` makes
//! it exit without opting in.

use std::time::Duration;

use manivelle::{
    Engine, EngineError, EngineOptions, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, TsSchema,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct AddInput {
    left: i32,
    right: i32,
}

#[derive(Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct AddOutput {
    value: i32,
}

struct Add;

impl HostContract for Add {
    const NAME: &'static str = "math.add";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Add {
    type Input = AddInput;
    type Output = AddOutput;
}

impl HostFunction for Add {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(AddOutput {
            value: input.left + input.right,
        })
    }
}

fn main() {
    if std::env::args().any(|arg| arg == "no-opt-in") {
        return;
    }
    if std::env::var_os("MANIVELLE_TEST_HANG").is_some() {
        std::thread::sleep(Duration::from_secs(30));
    }
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine.registry().function::<Add>().expect("register");
    // Writes the SDK when the tool asked for it; the program ends either way.
    engine.setup_protocol().expect("set up the protocol");
}
