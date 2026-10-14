//! Error types exposed by the engine.

use std::io;

use thiserror::Error;

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
    /// [`EngineOptions`](crate::EngineOptions) hold a value that would disable a protection
    /// instead of setting it.
    #[error("invalid engine options: {details}")]
    InvalidOptions {
        /// Which option, and why it is refused.
        details: String,
    },
    /// The requested script could not be found.
    #[error("script `{script_id}` is not loaded")]
    ScriptNotFound {
        /// Missing script identifier.
        script_id: String,
    },
    /// The requested export is missing or is not callable.
    #[error("function `{function_name}` is not exported by script `{script_id}`")]
    FunctionNotFound {
        /// Script identifier.
        script_id: String,
        /// Exported function name.
        function_name: String,
    },
    /// A value crossing the host bridge does not match its registered contract schema.
    #[error("host contract `{contract_name}` {direction} validation failed: {details}")]
    ContractValidation {
        /// Host contract stable name.
        contract_name: String,
        /// Bridge direction being validated.
        direction: &'static str,
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
    /// Filesystem operation failed.
    #[error("filesystem error: {0}")]
    Io(#[from] io::Error),
}
