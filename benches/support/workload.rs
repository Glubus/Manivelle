//! Host contracts shared by `benches/vs_lua.rs` and `examples/native_roundtrip.rs`.
//! The example includes this file with `#[path]`, so it uses only the public API.

use manivelle::{
    EngineError, HostCallback, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
};
use serde_json::Value;

/// `math.inc(x)`: adds one. Scripts call it from their loops.
pub struct Inc;

impl HostContract for Inc {
    const NAME: &'static str = "math.inc";
    const IMPORT_MODULE: &'static str = "bench";
    const EXPORT_PATH: &'static [&'static str] = &["math", "inc"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Inc {
    type Input = f64;
    type Output = f64;
}

impl HostFunction for Inc {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(input + 1.0)
    }
}

/// `score.onUpdate(handler)`: the host delivers `{ combo }` events to scripts.
pub struct ScoreUpdate;

impl HostContract for ScoreUpdate {
    const NAME: &'static str = "score.update";
    const IMPORT_MODULE: &'static str = "bench";
    const EXPORT_PATH: &'static [&'static str] = &["score", "onUpdate"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for ScoreUpdate {
    type Payload = Value;
}
