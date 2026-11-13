//! QuickJS error mapping helpers.

use rquickjs::CatchResultExt;

use crate::contract::JsDecode;
use crate::error::EngineError;

/// The message of QuickJS's own out of memory, an `InternalError`.
const OUT_OF_MEMORY: &str = "out of memory";

/// An rquickjs error as the operation's error. An allocation rquickjs failed is
/// [`EngineError::OutOfMemory`], like QuickJS's own, so the engine reports them alike
/// wherever the allocation failed.
pub(crate) fn js_error(error: rquickjs::Error) -> EngineError {
    match error {
        rquickjs::Error::Allocation => EngineError::OutOfMemory,
        error => EngineError::Execution {
            details: error.to_string(),
        },
    }
}

/// The exception QuickJS left pending in `ctx` after a call through the C API returned
/// the exception marker, as the operation's error. It clears the pending exception.
pub(crate) fn pending_exception(ctx: &rquickjs::Ctx<'_>) -> EngineError {
    use rquickjs::CatchResultExt;
    Err::<(), _>(rquickjs::Error::Exception)
        .catch(ctx)
        .map_or_else(caught_js_error, |()| EngineError::Execution {
            details: "an exception without a value".to_owned(),
        })
}

/// What a script threw, as the operation's error. A real out of memory is told by its
/// type, QuickJS's `InternalError`: its message reads the same as a script's own
/// `throw new Error("out of memory")`, which stays an ordinary error.
pub(crate) fn caught_js_error(error: rquickjs::CaughtError<'_>) -> EngineError {
    let details = match error {
        rquickjs::CaughtError::Exception(exception) if is_quickjs_out_of_memory(&exception) => {
            return EngineError::OutOfMemory;
        }
        rquickjs::CaughtError::Exception(exception) => exception_details(&exception),
        rquickjs::CaughtError::Value(value) => js_value_details(&value),
        rquickjs::CaughtError::Error(error) => return js_error(error),
    };
    EngineError::Execution { details }
}

/// Decodes what a script returned to the host as `R`. A value of the wrong shape is an
/// [`EngineError::OutputDecoding`]; a QuickJS failure met while reading it (an exception,
/// an allocation) is reported as any other.
pub(crate) fn decode_returned<'js, R: JsDecode>(
    ctx: &rquickjs::Ctx<'js>,
    value: rquickjs::Value<'js>,
) -> Result<R, EngineError> {
    R::decode_js(ctx, value).map_err(|error| match error.into_mismatch() {
        Ok(details) => EngineError::OutputDecoding { details },
        Err(error) => match Err::<std::convert::Infallible, _>(error).catch(ctx) {
            Err(caught) => caught_js_error(caught),
            Ok(never) => match never {},
        },
    })
}

/// Describes a thrown or rejected JavaScript value, with its stack when it is an `Error`.
pub(crate) fn js_value_details(value: &rquickjs::Value<'_>) -> String {
    match value
        .as_object()
        .and_then(|object| rquickjs::Exception::from_object(object.clone()))
    {
        Some(exception) => exception_details(&exception),
        // `null` keeps its debug form: the engine recognises the one QuickJS throws when it
        // runs out of memory by it.
        None if value.is_null() => format!("non-error exception: {value:?}"),
        None => format!("non-error exception: {}", thrown_text(value)),
    }
}

/// A thrown value that is not an `Error`, as the JSON it stringifies to: the debug form of
/// an object is its address, which tells nobody what was thrown. Without a JSON form (a
/// function, a symbol, a cycle) it is the debug form.
fn thrown_text(value: &rquickjs::Value<'_>) -> String {
    let ctx = value.ctx();
    match ctx.json_stringify(value.clone()) {
        Ok(Some(json)) => json.to_string().unwrap_or_else(|_| format!("{value:?}")),
        Ok(None) => format!("{value:?}"),
        Err(_) => {
            // The failed stringify left its exception pending.
            drop(ctx.catch());
            format!("{value:?}")
        }
    }
}

fn exception_details(exception: &rquickjs::Exception<'_>) -> String {
    let message = exception
        .message()
        .unwrap_or_else(|| String::from("javascript exception"));
    match exception.stack() {
        Some(stack) if !stack.trim().is_empty() => format!("{message}\n{stack}"),
        _ => message,
    }
}

/// QuickJS's own out of memory, an `InternalError`: the same words in a script's
/// `throw new Error("out of memory")` are an ordinary error.
fn is_quickjs_out_of_memory(exception: &rquickjs::Exception<'_>) -> bool {
    exception.message().as_deref() == Some(OUT_OF_MEMORY)
        && exception
            .as_object()
            .get::<_, Option<rquickjs::convert::Coerced<String>>>("name")
            .ok()
            .flatten()
            .is_some_and(|name| name.0 == "InternalError")
}
