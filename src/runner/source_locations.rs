//! Maps the runtime module locations in error details back to the TypeScript source.

use std::borrow::Cow;

use super::module_loader::{ModuleStore, RUNTIME_MODULE_PREFIX};
use crate::error::EngineError;

/// Points the JavaScript locations of an execution error at the TypeScript source;
/// see [`locations_in_typescript`].
pub(crate) fn in_typescript(error: EngineError, modules: &ModuleStore) -> EngineError {
    match error {
        EngineError::Execution { details } => {
            let mapped = match locations_in_typescript(&details, modules) {
                Cow::Owned(mapped) => Some(mapped),
                Cow::Borrowed(_) => None,
            };
            EngineError::Execution {
                details: mapped.unwrap_or(details),
            }
        }
        error => error,
    }
}

/// `text` with each `manivelle://graph/...:line:column` location, in a message or a stack
/// frame, that falls in a loaded script module replaced by `path:line:column` in its
/// TypeScript file. Other locations stay as they are; text without any is borrowed.
pub(crate) fn locations_in_typescript<'a>(text: &'a str, modules: &ModuleStore) -> Cow<'a, str> {
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
fn typescript_line<'a>(line: &'a str, modules: &ModuleStore) -> Cow<'a, str> {
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
fn typescript_location(location: &str, modules: &ModuleStore) -> Option<String> {
    let (position, column) = location.rsplit_once(':')?;
    let (module_id, line) = position.rsplit_once(':')?;
    modules.locate(module_id, line.parse().ok()?, column.parse().ok()?)
}
