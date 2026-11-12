//! Error types exposed by the engine.

use std::fmt;
use std::io;
use std::time::Duration;

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
    /// Code without permissions was loaded under the id of a script that came from a
    /// package: replacing it would lift the package's `permissions`. Unload it first.
    #[error(
        "script `{script_id}` was loaded from a package with permissions: call `unload_script` before loading other code under its id"
    )]
    ScriptRestricted {
        /// The id of the package's script.
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
    /// An operation ran longer than
    /// [`EngineOptions::execution_timeout`](crate::EngineOptions::execution_timeout) and
    /// was stopped: a call, a load, a `ctx.hot` callback, or the Promise jobs an
    /// operation left to settle. A script that runs out of its time while an event, a
    /// timer or a host answer is delivered to it is a [`EngineError::Fault`] instead,
    /// which names it.
    #[error("execution budget exceeded ({timeout:?}, `EngineOptions::execution_timeout`)")]
    BudgetExceeded {
        /// The budget the operation had.
        timeout: Duration,
    },
    /// QuickJS could not allocate: the engine reached
    /// [`EngineOptions::memory_limit_bytes`](crate::EngineOptions::memory_limit_bytes).
    /// [`Engine::memory_stats`](crate::Engine::memory_stats) has the numbers. A script's
    /// own `throw new Error("out of memory")` is an [`EngineError::Execution`].
    #[error(
        "out of memory: the engine reached `EngineOptions::memory_limit_bytes`; raise it or load fewer scripts"
    )]
    OutOfMemory,
    /// A script ran out of its time or memory. While an event, a timer or a host answer
    /// is delivered, the fault is counted against the script, and `standing` says how
    /// close it is to quarantine; a call that goes over the script's memory quota is
    /// reported the same way and not counted.
    #[error("{what} of script `{script_id}` {fault}{standing}")]
    Fault {
        /// The script at fault.
        script_id: ScriptId,
        /// What ran when it faulted, such as ``a `tick` handler`` or `a timer`.
        what: String,
        /// What it ran out of.
        fault: ScriptFault,
        /// Where the script stands now.
        standing: FaultStanding,
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

/// What a script ran out of, in an [`EngineError::Fault`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ScriptFault {
    /// Its time: the slice of the operation it had to itself.
    Budget {
        /// The budget it had
        /// ([`EngineOptions::execution_timeout`](crate::EngineOptions::execution_timeout)).
        timeout: Duration,
    },
    /// The engine's memory, which it was the first to fail to allocate from.
    Memory,
    /// Its own memory quota.
    Quota {
        /// The quota, in bytes
        /// ([`EngineOptions::script_memory_limit_bytes`](crate::EngineOptions::script_memory_limit_bytes)).
        limit_bytes: usize,
    },
}

impl fmt::Display for ScriptFault {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Budget { timeout } => {
                write!(f, "ran out of its execution budget ({timeout:?})")
            }
            Self::Memory => f.write_str("ran out of memory"),
            Self::Quota { limit_bytes } => write!(
                f,
                "went over its memory quota ({limit_bytes} bytes, `EngineOptions::script_memory_limit_bytes`)"
            ),
        }
    }
}

/// Where a script stands after an [`EngineError::Fault`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum FaultStanding {
    /// No quarantine follows: `EngineOptions::max_script_faults` is 0, or the fault
    /// happened in a call, which is not counted.
    Unlimited,
    /// It has `faults` of the `allowed` ones.
    Warned {
        /// The faults counted against it so far.
        faults: u32,
        /// The count at which it is quarantined.
        allowed: u32,
    },
    /// It reached the limit and is quarantined: its handlers and timers no longer run
    /// until it is reloaded.
    Quarantined {
        /// The faults counted against it.
        faults: u32,
    },
}

impl fmt::Display for FaultStanding {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Unlimited => Ok(()),
            Self::Warned { faults, allowed } => {
                write!(f, "; fault {faults} of {allowed} before it is quarantined")
            }
            Self::Quarantined { faults } => write!(
                f,
                "; it faulted {faults} times and is quarantined: its handlers and timers no \
                 longer run until it is reloaded (unload it to free its memory)"
            ),
        }
    }
}

/// The direction in which a value crosses the host bridge.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Direction {
    /// From the script into the host: a function's input.
    Input,
    /// From the host back into the script: a function's output.
    Output,
}

impl fmt::Display for Direction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Input => "input",
            Self::Output => "output",
        })
    }
}
