//! Host function contract traits.

use super::{HostContract, JsDecode, JsEncode, TsSchema};
use crate::error::EngineError;
use crate::registry::InMemoryHostContractRegistry;

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

/// A host function that registers itself, handler included: what
/// [`InMemoryHostContractRegistry::function`](crate::InMemoryHostContractRegistry::function)
/// requires of its type parameter.
///
/// Every [`HostFunction`], hand-written or declared with `#[host_fn]`, implements it. So
/// does the marker of a `#[host_fn]` function that takes the `Caller` or a `HostResolver`,
/// whose handler needs more than a static `call`: the macro writes the impl. Register
/// through `registry.function::<T>()` rather than calling [`Self::register_in`].
#[diagnostic::on_unimplemented(
    message = "`{Self}` cannot be registered with `InMemoryHostContractRegistry::function`",
    label = "not a `HostFunction`, nor declared with `#[host_fn]`",
    note = "its `Input` must implement `TsSchema` and `JsDecode`, and its `Output` `TsSchema` and `JsEncode`; to register a closure, use `function_with`"
)]
pub trait RegisterHostFunction {
    /// Registers this function, with its handler, in `registry`.
    ///
    /// # Errors
    ///
    /// The refusals of [`InMemoryHostContractRegistry::function`](crate::InMemoryHostContractRegistry::function).
    fn register_in(
        registry: &InMemoryHostContractRegistry,
    ) -> Result<&InMemoryHostContractRegistry, EngineError>;
}

impl<T> RegisterHostFunction for T
where
    T: HostFunction + 'static,
    T::Input: TsSchema + JsDecode,
    T::Output: TsSchema + JsEncode,
{
    fn register_in(
        registry: &InMemoryHostContractRegistry,
    ) -> Result<&InMemoryHostContractRegistry, EngineError> {
        registry.function_with::<T>(T::call)
    }
}
