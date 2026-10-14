//! Host function contract traits.

use super::HostContract;
use crate::error::EngineError;

/// Input and output of a host function: what scripts see, whatever implements it.
///
/// A contract implementing only this trait is registered together with a handler
/// value, such as a closure holding state
/// ([`InMemoryHostContractRegistry::function_with`](crate::InMemoryHostContractRegistry::function_with));
/// a contract implementing [`HostFunction`] as well is registered on its own.
pub trait HostFunctionSignature: HostContract {
    /// Function input type.
    type Input;
    /// Function output type.
    type Output;
}

/// Host function implemented by a static Rust function.
pub trait HostFunction: HostFunctionSignature {
    /// Executes the host function from Rust.
    fn call(input: Self::Input) -> Result<Self::Output, EngineError>;
}
