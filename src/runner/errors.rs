//! QuickJS error mapping helpers.

use std::cell::Cell;

use crate::error::EngineError;

/// How every out of memory starts once described: QuickJS's own `InternalError`, an
/// allocation rquickjs failed ([`rquickjs::Error::Allocation`]) and a context the engine
/// could not create, so the engine reports them alike wherever the allocation failed.
pub(crate) const OUT_OF_MEMORY: &str = "out of memory";

thread_local! {
    /// Set by describing an error that was a real out of memory. The engine runs on one
    /// thread, and the error it describes is `EngineError::Execution { details }` whatever
    /// it was, so what the details cannot carry is kept here.
    static RAISED_OUT_OF_MEMORY: Cell<bool> = const { Cell::new(false) };
}

pub(crate) fn js_error(error: rquickjs::Error) -> EngineError {
    EngineError::Execution {
        details: rquickjs_error_details(&error),
    }
}

fn rquickjs_error_details(error: &rquickjs::Error) -> String {
    match error {
        rquickjs::Error::Allocation => {
            RAISED_OUT_OF_MEMORY.set(true);
            OUT_OF_MEMORY.to_owned()
        }
        error => error.to_string(),
    }
}

/// Forgets the out of memory [`raised_out_of_memory`] reports: an operation or a slice
/// starts, and what it raises is its own.
pub(crate) fn forget_out_of_memory() {
    RAISED_OUT_OF_MEMORY.set(false);
}

/// Whether an error described since the last [`forget_out_of_memory`] was a real out of
/// memory, told by its type: QuickJS's `InternalError` or an allocation rquickjs failed.
/// Its details read the same as a script's own `throw new Error("out of memory")`, so
/// they cannot say.
pub(crate) fn raised_out_of_memory() -> bool {
    RAISED_OUT_OF_MEMORY.get()
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

pub(crate) fn caught_js_error(error: rquickjs::CaughtError<'_>) -> EngineError {
    EngineError::Execution {
        details: caught_js_error_details(&error),
    }
}

pub(crate) fn caught_js_error_details(error: &rquickjs::CaughtError<'_>) -> String {
    match error {
        rquickjs::CaughtError::Exception(exception) => exception_details(exception),
        rquickjs::CaughtError::Value(value) => js_value_details(value),
        rquickjs::CaughtError::Error(error) => rquickjs_error_details(error),
    }
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
    if is_quickjs_out_of_memory(exception, &message) {
        RAISED_OUT_OF_MEMORY.set(true);
    }
    match exception.stack() {
        Some(stack) if !stack.trim().is_empty() => format!("{message}\n{stack}"),
        _ => message,
    }
}

/// QuickJS's own out of memory, an `InternalError`: the same words in a script's
/// `throw new Error("out of memory")` are an ordinary error.
fn is_quickjs_out_of_memory(exception: &rquickjs::Exception<'_>, message: &str) -> bool {
    message == OUT_OF_MEMORY
        && exception
            .as_object()
            .get::<_, Option<rquickjs::convert::Coerced<String>>>("name")
            .ok()
            .flatten()
            .is_some_and(|name| name.0 == "InternalError")
}
