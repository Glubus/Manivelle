use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use rquickjs::{Ctx, Exception, Object, Result as JsResult, Value as JsValue};

use crate::contract::validation::Snapshot;
use crate::contract::{
    Caller, HostFunctionSignature, HostResolver, JsDecode, JsEncode,
    js_value_to_validation_snapshot,
};
use crate::error::EngineError;
use crate::packages::PermissionSet;
use crate::runner::execution::ExecutionControl;
use crate::runner::host_fn::{HostCall, HostFnClass};
use crate::runner::host_promises::WeakHostPromises;
use crate::runner::reply_inbox::{EncodeReply, ReplyValue};

/// The resolver an async handler of contract `C` receives.
type Resolver<C> = HostResolver<<C as HostFunctionSignature>::Output>;

/// The validation policy of one contract, when the registry validates any value.
pub(super) type Validator = Option<Arc<dyn ContractValidator>>;

/// A registered host function, sync or async.
#[derive(Clone)]
pub(super) enum Binding {
    Sync(Arc<dyn HostFunctionBinding>),
    Async(Arc<dyn AsyncFunctionBinding>),
}

/// What installing a native host function needs from the engine, besides the function.
pub struct NativeContext {
    /// The budget of the running operation, whose clock a call to the function starts.
    pub execution: Arc<ExecutionControl>,
    /// The callable class the function is an instance of.
    pub class: HostFnClass,
}

/// A registered host function handler.
pub(super) trait HostFunctionBinding: Send + Sync {
    /// Sets the handler on `target` as the native function `name` of script `script_id`,
    /// checking its values with `validator` when there is one. Entering it starts the
    /// clock of the running operation's budget in `native`.
    fn install<'js>(
        self: Arc<Self>,
        target: &Object<'js>,
        name: &str,
        script_id: &str,
        native: &NativeContext,
        validator: Validator,
    ) -> JsResult<()>;
}

/// A registered async host function handler: each call returns a Promise that a
/// [`HostResolver`] settles.
pub(super) trait AsyncFunctionBinding: Send + Sync {
    /// Sets the handler on `target` as the native function `name` of script
    /// `script_id`, whose calls start in `promises`, checking its values with
    /// `validator` when there is one.
    fn install<'js>(
        self: Arc<Self>,
        target: &Object<'js>,
        name: &str,
        script_id: &Rc<str>,
        promises: &WeakHostPromises,
        native: &NativeContext,
        validator: Validator,
    ) -> JsResult<()>;
}

/// Checks the values of one contract as the registry's validation policy asks. It sees
/// a JSON snapshot of each value, never the Rust type.
pub(super) trait ContractValidator: Send + Sync {
    /// Whether inputs are checked at all, so the snapshot is only taken when needed.
    fn checks_input(&self) -> bool;

    /// Whether outputs are checked at all.
    fn checks_output(&self) -> bool;

    /// Checks a call's input before the handler runs.
    fn validate_input(&self, input: &Snapshot) -> Result<(), EngineError>;

    /// Checks a handler's output before the script receives it.
    fn validate_output(&self, output: &Snapshot) -> Result<(), EngineError>;
}

/// Checks the JavaScript `input` of a call against `validator`, if it checks inputs.
fn check_input<'js>(
    ctx: &Ctx<'js>,
    validator: Option<&dyn ContractValidator>,
    input: &JsValue<'js>,
) -> JsResult<()> {
    let Some(validator) = validator.filter(|validator| validator.checks_input()) else {
        return Ok(());
    };
    validator
        .validate_input(&js_value_to_validation_snapshot(ctx, input.clone())?)
        .map_err(js_host_error)
}

/// Checks the JavaScript `output` of a call against `validator`, if it checks outputs.
fn check_output<'js>(
    ctx: &Ctx<'js>,
    validator: Option<&dyn ContractValidator>,
    output: &JsValue<'js>,
) -> JsResult<()> {
    let Some(validator) = validator.filter(|validator| validator.checks_output()) else {
        return Ok(());
    };
    validator
        .validate_output(&js_value_to_validation_snapshot(ctx, output.clone())?)
        .map_err(js_host_error)
}

/// Converts the call's input, runs `handler` on it and converts its output back, both
/// natively through [`JsDecode`] and [`JsEncode`]. The input and the output are checked
/// against `validator` when it checks them.
fn call_validated<'js, C>(
    ctx: &Ctx<'js>,
    input: JsValue<'js>,
    validator: Option<&dyn ContractValidator>,
    handler: impl FnOnce(C::Input) -> Result<C::Output, EngineError>,
) -> JsResult<JsValue<'js>>
where
    C: HostFunctionSignature,
    C::Input: JsDecode,
    C::Output: JsEncode,
{
    check_input(ctx, validator, &input)?;
    let input = C::Input::decode_js(ctx, input)?;
    let output = handler(input).map_err(js_host_error)?.encode_js(ctx)?;
    check_output(ctx, validator, &output)?;
    Ok(output)
}

/// A resolved output converted natively through [`JsEncode`].
struct NativeReply<T>(T);

impl<T: JsEncode + Send> ReplyValue for NativeReply<T> {
    fn into_js<'js>(self: Box<Self>, ctx: &Ctx<'js>) -> JsResult<JsValue<'js>> {
        self.0.encode_js(ctx)
    }
}

/// A resolved output checked against its contract once converted, on the engine thread.
/// A failed check rejects the Promise with the validation message.
struct ValidatedReply<T> {
    output: T,
    validator: Arc<dyn ContractValidator>,
}

impl<T: JsEncode + Send> ReplyValue for ValidatedReply<T> {
    fn into_js<'js>(self: Box<Self>, ctx: &Ctx<'js>) -> JsResult<JsValue<'js>> {
        let output = self.output.encode_js(ctx)?;
        let snapshot = js_value_to_validation_snapshot(ctx, output.clone())?;
        self.validator
            .validate_output(&snapshot)
            .map_err(|error| Exception::throw_message(ctx, &error.to_string()))?;
        Ok(output)
    }
}

/// Handler of contract `C` that does not ask which script called it.
struct PlainBinding<C, F> {
    handler: F,
    marker: PhantomData<fn() -> C>,
}

impl<C, F> HostFunctionBinding for PlainBinding<C, F>
where
    C: HostFunctionSignature + 'static,
    C::Input: JsDecode,
    C::Output: JsEncode,
    F: Fn(C::Input) -> Result<C::Output, EngineError> + Send + Sync + 'static,
{
    fn install<'js>(
        self: Arc<Self>,
        target: &Object<'js>,
        name: &str,
        _script_id: &str,
        native: &NativeContext,
        validator: Validator,
    ) -> JsResult<()> {
        let execution = Arc::clone(&native.execution);
        let class = native.class;
        let call: HostCall = Box::new(move |ctx, input| {
            execution.start_clock();
            call_validated::<C>(ctx, input, validator.as_deref(), &self.handler)
        });
        target.set(name, class.function(target.ctx(), call)?)
    }
}

/// Handler of contract `C` that receives the calling script as a [`Caller`].
struct CallerBinding<C, F> {
    handler: F,
    marker: PhantomData<fn() -> C>,
}

impl<C, F> HostFunctionBinding for CallerBinding<C, F>
where
    C: HostFunctionSignature + 'static,
    C::Input: JsDecode,
    C::Output: JsEncode,
    F: Fn(&Caller<'_>, C::Input) -> Result<C::Output, EngineError> + Send + Sync + 'static,
{
    /// The script id is copied once here, so a call only borrows it.
    fn install<'js>(
        self: Arc<Self>,
        target: &Object<'js>,
        name: &str,
        script_id: &str,
        native: &NativeContext,
        validator: Validator,
    ) -> JsResult<()> {
        let script_id = Box::<str>::from(script_id);
        let execution = Arc::clone(&native.execution);
        let class = native.class;
        let call: HostCall = Box::new(move |ctx, input| {
            execution.start_clock();
            let caller = Caller::new(&script_id);
            call_validated::<C>(ctx, input, validator.as_deref(), |input| {
                (self.handler)(&caller, input)
            })
        });
        target.set(name, class.function(target.ctx(), call)?)
    }
}

/// Async handler of contract `C`. A handler that does not ask which script called it is
/// stored wrapped to ignore the [`Caller`].
struct AsyncBinding<C: HostFunctionSignature, F> {
    handler: F,
    /// Keeps a resolved output until the engine thread converts it.
    encode: EncodeReply<C::Output>,
}

impl<C, F> AsyncFunctionBinding for AsyncBinding<C, F>
where
    C: HostFunctionSignature + 'static,
    C::Input: JsDecode,
    C::Output: JsEncode + Send,
    F: Fn(&Caller<'_>, C::Input, Resolver<C>) -> Result<(), EngineError> + Send + Sync + 'static,
{
    /// The function holds the scope weakly, so it neither keeps the scope's Promises
    /// alive nor starts calls once the scope is cancelled.
    fn install<'js>(
        self: Arc<Self>,
        target: &Object<'js>,
        name: &str,
        script_id: &Rc<str>,
        promises: &WeakHostPromises,
        native: &NativeContext,
        validator: Validator,
    ) -> JsResult<()> {
        let script_id = Rc::clone(script_id);
        let promises = promises.clone();
        let execution = Arc::clone(&native.execution);
        let class = native.class;
        let encode: EncodeReply<C::Output> = match validator.as_ref() {
            Some(validator) if validator.checks_output() => {
                let validator = Arc::clone(validator);
                Arc::new(move |output: C::Output| {
                    Ok(Box::new(ValidatedReply {
                        output,
                        validator: Arc::clone(&validator),
                    }) as Box<dyn ReplyValue>)
                })
            }
            _ => Arc::clone(&self.encode),
        };
        let call: HostCall = Box::new(move |ctx, input| {
            execution.start_clock();
            let caller = Caller::new(&script_id);
            promises.call(
                ctx,
                C::NAME,
                &encode,
                || {
                    check_input(ctx, validator.as_deref(), &input)?;
                    C::Input::decode_js(ctx, input)
                },
                |input, resolver| (self.handler)(&caller, input, resolver),
            )
        });
        target.set(name, class.function(target.ctx(), call)?)
    }
}

/// The error of registering a second contract under `name`, which the registry refuses so
/// that a name always means one implementation.
pub(super) fn duplicate_contract(name: &str) -> EngineError {
    EngineError::InvalidOptions {
        details: format!("host contract `{name}` is already registered"),
    }
}

fn js_host_error(error: impl ToString) -> rquickjs::Error {
    rquickjs::Error::new_from_js_message("host", "function", error.to_string())
}

#[derive(Default)]
pub(super) struct FunctionBindingStore {
    /// Shared, so that a script install takes its snapshot in constant time. An insert
    /// copies the map only while a snapshot is still held.
    pub(super) by_name: Mutex<Arc<HashMap<String, Binding>>>,
}

impl FunctionBindingStore {
    /// Stores `handler` as the implementation of contract `C`.
    pub(super) fn insert_plain<C>(
        &self,
        handler: impl Fn(C::Input) -> Result<C::Output, EngineError> + Send + Sync + 'static,
    ) -> Result<(), EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: JsDecode,
        C::Output: JsEncode,
    {
        self.insert(
            C::NAME,
            Binding::Sync(Arc::new(PlainBinding::<C, _> {
                handler,
                marker: PhantomData,
            })),
        )
    }

    /// Stores `handler`, which receives the calling script, as the implementation of
    /// contract `C`.
    pub(super) fn insert_with_caller<C>(
        &self,
        handler: impl Fn(&Caller<'_>, C::Input) -> Result<C::Output, EngineError>
        + Send
        + Sync
        + 'static,
    ) -> Result<(), EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: JsDecode,
        C::Output: JsEncode,
    {
        self.insert(
            C::NAME,
            Binding::Sync(Arc::new(CallerBinding::<C, _> {
                handler,
                marker: PhantomData,
            })),
        )
    }

    /// Stores async `handler`, which receives the calling script, as the implementation
    /// of contract `C`.
    pub(super) fn insert_async<C, F>(&self, handler: F) -> Result<(), EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: JsDecode,
        C::Output: JsEncode + Send,
        F: Fn(&Caller<'_>, C::Input, Resolver<C>) -> Result<(), EngineError>
            + Send
            + Sync
            + 'static,
    {
        self.insert(
            C::NAME,
            Binding::Async(Arc::new(AsyncBinding::<C, F> {
                handler,
                encode: Arc::new(|output: C::Output| {
                    Ok(Box::new(NativeReply(output)) as Box<dyn ReplyValue>)
                }),
            })),
        )
    }

    fn insert(&self, name: &str, binding: Binding) -> Result<(), EngineError> {
        let mut guard = self.by_name.lock().map_err(|_| EngineError::LockPoisoned)?;
        if guard.contains_key(name) {
            return Err(duplicate_contract(name));
        }
        Arc::make_mut(&mut *guard).insert(name.to_owned(), binding);
        Ok(())
    }

    /// Sets one native QuickJS function per binding on `target`, the host functions of
    /// script `script_id`, whose async calls start in `promises`. Each function owns
    /// its binding, so a call performs no name lookup and takes no lock, and the
    /// registry lock is released before any handler runs. `validator_for` gives the
    /// validator of each function by name, `None` where its values are not checked.
    pub(super) fn install_native<'js>(
        &self,
        target: &Object<'js>,
        script_id: &str,
        promises: &WeakHostPromises,
        native: &NativeContext,
        allowed: Option<&PermissionSet>,
        validator_for: impl Fn(&str) -> Validator,
    ) -> JsResult<()> {
        let bindings = self.snapshot().map_err(js_host_error)?;
        let mut shared_id = None;
        for (name, binding) in bindings.iter() {
            if allowed.is_some_and(|set| !set.allows(name)) {
                let message = EngineError::PermissionDenied {
                    package: script_id.to_owned(),
                    permission: name.clone(),
                }
                .to_string();
                // The function stays, so the script's types hold; calling it throws.
                let denied: HostCall =
                    Box::new(move |ctx, _input| Err(Exception::throw_message(ctx, &message)));
                target.set(name.as_str(), native.class.function(target.ctx(), denied)?)?;
                continue;
            }
            let validator = validator_for(name);
            match binding {
                Binding::Sync(binding) => {
                    Arc::clone(binding).install(target, name, script_id, native, validator)?;
                }
                Binding::Async(binding) => {
                    let shared_id = shared_id.get_or_insert_with(|| Rc::<str>::from(script_id));
                    Arc::clone(binding)
                        .install(target, name, shared_id, promises, native, validator)?;
                }
            }
        }
        Ok(())
    }

    /// The bindings as they are now: a shared copy, which later inserts do not change.
    fn snapshot(&self) -> Result<Arc<HashMap<String, Binding>>, EngineError> {
        let guard = self.by_name.lock().map_err(|_| EngineError::LockPoisoned)?;
        Ok(Arc::clone(&guard))
    }
}
