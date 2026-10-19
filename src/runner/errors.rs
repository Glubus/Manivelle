//! QuickJS error mapping helpers.

use crate::error::EngineError;

/// How every out of memory starts once described: QuickJS's own `InternalError`, an
/// allocation rquickjs failed ([`rquickjs::Error::Allocation`]) and a context the engine
/// could not create, so the engine reports them alike wherever the allocation failed.
pub(crate) const OUT_OF_MEMORY: &str = "out of memory";

pub(crate) fn js_error(error: rquickjs::Error) -> EngineError {
    EngineError::Execution {
        details: rquickjs_error_details(&error),
    }
}

fn rquickjs_error_details(error: &rquickjs::Error) -> String {
    match error {
        rquickjs::Error::Allocation => OUT_OF_MEMORY.to_owned(),
        error => error.to_string(),
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
        None => format!("non-error exception: {value:?}"),
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
