//! Calling a script's function from the host with the arguments of a [`JsArgs`].

use rquickjs::{CatchResultExt, Ctx, Function, Value as JsValue, qjs};

use super::super::errors::{caught_js_error, js_error, pending_exception};
use crate::contract::JsArgs;
use crate::error::EngineError;

/// The most scalar arguments passed without building a wrapper per argument.
const SCALAR_ARGUMENTS: usize = 8;

/// Calls `function` with `args` and returns what it returned, an owned QuickJS value the
/// caller must release (wrap it in a [`JsValue`], or drop it unread when it is a scalar).
///
/// A list of numbers and booleans (the usual per-frame call) is built on the stack and
/// passed borrowed: no argument is wrapped, so no context reference is taken per argument
/// and per result. Any other list is encoded and passed as rquickjs does.
pub(super) fn invoke<'js>(
    ctx: &Ctx<'js>,
    function: &Function<'js>,
    args: &impl JsArgs,
) -> Result<qjs::JSValue, EngineError> {
    let raw_ctx = ctx.as_raw().as_ptr();
    let mut scalars = [qjs::JS_UNDEFINED; SCALAR_ARGUMENTS];
    if let Some(count) = args.encode_scalars(&mut scalars) {
        // SAFETY: `function` is a live function of `ctx`; `scalars[..count]` holds numbers
        // and booleans, which own nothing, and `JS_Call` borrows them. `this` is
        // `undefined`, which needs no release.
        let returned = unsafe {
            qjs::JS_Call(
                raw_ctx,
                function.as_value().as_raw(),
                qjs::JS_UNDEFINED,
                count as _,
                scalars.as_mut_ptr(),
            )
        };
        // SAFETY: reading the tag of the value `JS_Call` returned.
        return if unsafe { qjs::JS_IsException(returned) } {
            Err(pending_exception(ctx))
        } else {
            Ok(returned)
        };
    }
    let args = args.encode_args(ctx).map_err(js_error)?;
    let returned = function
        .call_arg::<JsValue<'_>>(args)
        .catch(ctx)
        .map_err(caught_js_error)?;
    // SAFETY: `returned` is a value of `ctx`; the duplicate is the reference handed to the
    // caller, and the wrapper releases its own as it drops.
    Ok(unsafe { qjs::JS_DupValue(raw_ctx, returned.as_raw()) })
}
