//! QuickJS error mapping helpers.

use std::borrow::Cow;

use crate::error::VmError;

use super::module_loader::{RUNTIME_MODULE_PREFIX, WorkerModuleStore};

/// How every out of memory starts once described: QuickJS's own `InternalError`, an
/// allocation rquickjs failed ([`rquickjs::Error::Allocation`]) and a context the engine
/// could not create, so the engine reports them alike wherever the allocation failed.
pub(crate) const OUT_OF_MEMORY: &str = "out of memory";

pub(crate) fn js_error(error: rquickjs::Error) -> VmError {
    VmError::Execution {
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
pub(crate) fn pending_exception(ctx: &rquickjs::Ctx<'_>) -> VmError {
    use rquickjs::CatchResultExt;
    Err::<(), _>(rquickjs::Error::Exception)
        .catch(ctx)
        .map_or_else(caught_js_error, |()| VmError::Execution {
            details: "an exception without a value".to_owned(),
        })
}

pub(crate) fn caught_js_error(error: rquickjs::CaughtError<'_>) -> VmError {
    VmError::Execution {
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

/// Points the JavaScript locations of an execution error at the TypeScript source;
/// see [`locations_in_typescript`].
pub(crate) fn in_typescript(error: VmError, modules: &WorkerModuleStore) -> VmError {
    match error {
        VmError::Execution { details } => {
            let mapped = match locations_in_typescript(&details, modules) {
                Cow::Owned(mapped) => Some(mapped),
                Cow::Borrowed(_) => None,
            };
            VmError::Execution {
                details: mapped.unwrap_or(details),
            }
        }
        error => error,
    }
}

/// `text` with each `manivelle://graph/...:line:column` location, in a message or a stack
/// frame, that falls in a loaded script module replaced by `path:line:column` in its
/// TypeScript file. Other locations stay as they are; text without any is borrowed.
pub(crate) fn locations_in_typescript<'a>(
    text: &'a str,
    modules: &WorkerModuleStore,
) -> Cow<'a, str> {
    if !text.contains(RUNTIME_MODULE_PREFIX) {
        return Cow::Borrowed(text);
    }
    Cow::Owned(
        text.split_inclusive('\n')
            .map(|line| typescript_line(line, modules))
            .collect(),
    )
}

/// One line of error details, with its runtime module location mapped when it has one.
fn typescript_line<'a>(line: &'a str, modules: &WorkerModuleStore) -> Cow<'a, str> {
    let Some(start) = line.find(RUNTIME_MODULE_PREFIX) else {
        return Cow::Borrowed(line);
    };
    let location = location_text(&line[start..]);
    match typescript_location(location, modules) {
        Some(typescript) => Cow::Owned(format!(
            "{}{typescript}{}",
            &line[..start],
            &line[start + location.len()..]
        )),
        None => Cow::Borrowed(line),
    }
}

/// The location `text` starts with: the rest of the line, without the `)` that closes a
/// stack frame.
fn location_text(text: &str) -> &str {
    let text = text.trim_end_matches(['\n', '\r']);
    text.strip_suffix(')').unwrap_or(text)
}

/// Maps `module id:line:column` to its TypeScript location.
fn typescript_location(location: &str, modules: &WorkerModuleStore) -> Option<String> {
    let (position, column) = location.rsplit_once(':')?;
    let (module_id, line) = position.rsplit_once(':')?;
    modules.locate(module_id, line.parse().ok()?, column.parse().ok()?)
}
