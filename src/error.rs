//! Error types exposed by the engine.

use std::io;

use thiserror::Error;

use crate::types::ScriptId;

/// Error type for all engine operations.
#[derive(Debug, Error)]
#[non_exhaustive]
pub enum EngineError {
    /// An [`InterruptHandle`](crate::InterruptHandle) stopped the running JavaScript.
    #[error("javascript execution was interrupted by the host")]
    Interrupted,
    /// The script generation owning a deferred operation was replaced or unloaded.
    #[error("deferred operation cancelled because its script was replaced or unloaded")]
    Cancelled,
    /// A lock was poisoned by a panic while it was held, typically in a host handler.
    #[error("a lock was poisoned by a panic in a host handler")]
    LockPoisoned,
    /// A host function asked for the host state outside an
    /// [`Engine::with_state`](crate::Engine::with_state) scope.
    #[error("no host state is lent: the call did not run inside `Engine::with_state`")]
    NoState,
    /// A host function asked for a host state type other than the one the scope lent.
    #[error("the host state lent by `Engine::with_state` is not a `{expected}`")]
    StateType {
        /// The type the host function asked for.
        expected: &'static str,
    },
    /// A host function asked for the host state while an enclosing host function holds it:
    /// two `&mut` to the same state cannot coexist.
    #[error("the host state is already borrowed by a host function further up the call stack")]
    StateBusy,
    /// [`Engine::with_state`](crate::Engine::with_state) was called inside a scope: states
    /// do not nest.
    #[error("`Engine::with_state` is already running: its scopes do not nest")]
    StateScopeActive,
    /// [`Engine::restore`](crate::Engine::restore) was given the state of a script that is
    /// already loaded: its code has run, so the state could no longer be its `ctx.hot.data`.
    #[error("script `{script_id}` is already loaded: restore its state before loading it")]
    ScriptAlreadyLoaded {
        /// The loaded script the snapshot holds a state for.
        script_id: ScriptId,
    },
    /// [`EngineOptions`](crate::EngineOptions) hold a value that would disable a protection
    /// instead of setting it.
    #[error("invalid engine options: {details}")]
    InvalidOptions {
        /// Which option, and why it is refused.
        details: String,
    },
    /// A host contract name is registered twice: a name stands for one contract.
    #[error("host contract `{name}` is already registered")]
    DuplicateContract {
        /// The contract name that is already taken.
        name: String,
    },
    /// The requested script could not be found.
    #[error("script `{script_id}` is not loaded")]
    ScriptNotFound {
        /// Missing script identifier.
        script_id: ScriptId,
    },
    /// The requested export is missing or is not callable.
    #[error("function `{function_name}` is not exported by script `{script_id}`")]
    FunctionNotFound {
        /// Script identifier.
        script_id: ScriptId,
        /// Exported function name.
        function_name: String,
    },
    /// A value crossing the host bridge does not match its registered contract schema.
    #[error("host contract `{contract_name}` {direction} validation failed: {details}")]
    ContractValidation {
        /// Host contract stable name.
        contract_name: String,
        /// Bridge direction being validated.
        direction: Direction,
        /// Validation failure details.
        details: String,
    },
    /// TypeScript transpilation failed.
    #[error("typescript transpilation failed: {details}")]
    Transpile {
        /// One `path:line:column: message` line per compiler diagnostic, followed by
        /// its labels and help, indented.
        details: String,
    },
    /// Project module resolution failed.
    #[error("module resolution failed: {details}")]
    Resolve {
        /// Resolution failure details.
        details: String,
    },
    /// JavaScript execution failed.
    #[error("javascript execution failed: {details}")]
    Execution {
        /// The error message and stack; script frames name their TypeScript file,
        /// line and column.
        details: String,
    },
    /// A package called a host function, or listened to an event, that its `permissions`
    /// do not list.
    #[error(
        "permission denied: package `{package}` needs `{permission}` in the `permissions` of its manivelle.kdl"
    )]
    PermissionDenied {
        /// The package.
        package: String,
        /// The contract it reached for.
        permission: String,
    },
    /// A package's `manivelle.kdl` manifest is malformed, or the set of packages cannot be
    /// loaded.
    #[error("invalid package manifest {path}: {details}")]
    Manifest {
        /// The manifest file, or the package it concerns.
        path: String,
        /// What is wrong, with the line and column when the text is at fault.
        details: String,
    },
    /// Filesystem operation failed.
    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),
}

/// The direction in which a value crosses the host bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// From the script into the host: a function's input.
    Input,
    /// From the host back into the script: a function's output.
    Output,
}

impl std::fmt::Display for Direction {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Input => "input",
            Self::Output => "output",
        })
    }
}
