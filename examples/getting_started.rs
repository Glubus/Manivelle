//! The program of the [Getting Started](../docs/getting-started.md) guide: one Rust
//! function exposed to a TypeScript script. It also opts in to `manivelle init`.

use manivelle::{
    Engine, EngineError, EngineOptions, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, SdkFileNames, TsSchema,
};
use serde::{Deserialize, Serialize};
use serde_json::Value;

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

fn main() -> Result<(), EngineError> {
    let mut engine = Engine::new(&EngineOptions::default())?;

    engine.registry().function::<Add>()?;
    // `manivelle init --from <this program>` asks for the declarations here.
    if engine.setup_protocol()?.exported {
        return Ok(());
    }
    engine.registry().write_sdk_files_with_names(
        "target/generated",
        &SdkFileNames {
            types: "my_sdk.d.ts".into(),
            sdk: "my_sdk.ts".into(),
        },
    )?;

    engine.load_script("math-script", include_str!("getting_started.ts"))?;

    let output: Value = engine.call("math-script", "run", ())?;
    println!("{output}");

    Ok(())
}
