use crate::contract::{
    Caller, HostCallback, HostContext, HostContractDescriptor, HostFunction, HostFunctionSignature,
    HostRequest, HostResolver, JsDecode, JsEncode, TsSchema,
};
use crate::error::EngineError;

/// Access to the host contract registry.
pub trait HostContractRegistry: Send + Sync {
    /// Registers one host function contract, implemented by its static
    /// [`HostFunction::call`].
    ///
    /// The schemas come from `TsSchema` of the input and output types. Script calls
    /// convert the input with [`JsDecode`] and the output with [`JsEncode`], natively
    /// and without JSON text.
    fn register_function<T>(&self) -> Result<(), EngineError>
    where
        T: HostFunction + 'static,
        T::Input: TsSchema + JsDecode,
        T::Output: TsSchema + JsEncode;

    /// Registers function contract `C` implemented by `handler`, a closure that can
    /// hold state; see [`Self::register_function`].
    fn register_function_with<C>(
        &self,
        handler: impl Fn(C::Input) -> Result<C::Output, EngineError> + Send + Sync + 'static,
    ) -> Result<(), EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode;

    /// Registers function contract `C` implemented by `handler`, which also receives
    /// the [`Caller`]; see [`Self::register_function_with`].
    fn register_function_with_caller<C>(
        &self,
        handler: impl Fn(&Caller<'_>, C::Input) -> Result<C::Output, EngineError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode;

    /// Registers async function contract `C` implemented by `handler`: scripts receive a
    /// `Promise` of the output, which the [`HostResolver`] passed to `handler` with each
    /// call's input settles. An `Err` from `handler` rejects the Promise at once. Values
    /// cross as in [`Self::register_function_with`]; the output is converted on the
    /// engine thread, so it must be `Send`.
    fn register_async_function_with<C>(
        &self,
        handler: impl Fn(C::Input, HostResolver<C::Output>) -> Result<(), EngineError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode + Send;

    /// Registers async function contract `C` implemented by `handler`, which also
    /// receives the [`Caller`]; see [`Self::register_async_function_with`].
    fn register_async_function_with_caller<C>(
        &self,
        handler: impl Fn(&Caller<'_>, C::Input, HostResolver<C::Output>) -> Result<(), EngineError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode + Send;

    /// Registers one host callback contract using `TsSchema` from its payload type.
    fn register_callback<T>(&self) -> Result<(), EngineError>
    where
        T: HostCallback + 'static,
        T::Payload: TsSchema + JsEncode;

    /// Registers one host request, a callback whose handlers reply, using `TsSchema` from
    /// its payload and reply types.
    fn register_request<T>(&self) -> Result<(), EngineError>
    where
        T: HostRequest + 'static,
        T::Payload: TsSchema + JsEncode,
        T::Reply: TsSchema + JsDecode;

    /// Registers one host context contract.
    fn register_context<T>(&self) -> Result<(), EngineError>
    where
        T: HostContext;

    /// Returns one descriptor by stable contract name.
    fn get(&self, name: &str) -> Result<Option<HostContractDescriptor>, EngineError>;

    /// Returns every stored descriptor.
    fn list(&self) -> Result<Vec<HostContractDescriptor>, EngineError>;

    /// Renders TypeScript declarations from registered ABI and schema metadata.
    fn typescript_declarations(&self) -> Result<String, EngineError>;

    /// Renders an ergonomic TypeScript SDK from registered ABI and schema metadata.
    fn typescript_sdk(&self) -> Result<String, EngineError>;
}
