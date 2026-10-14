//! Configuration types for the engine.

use std::path::PathBuf;
use std::time::Duration;

/// Host contract validation policy.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ContractValidation {
    /// Do not validate host function payloads against registered schemas.
    #[default]
    Disabled,
    /// Validate host function inputs before calling Rust handlers.
    Inputs,
    /// Validate host function inputs and outputs around Rust handlers.
    InputsAndOutputs,
}

impl ContractValidation {
    pub(crate) fn validates_inputs(self) -> bool {
        matches!(self, Self::Inputs | Self::InputsAndOutputs)
    }

    pub(crate) fn validates_outputs(self) -> bool {
        matches!(self, Self::InputsAndOutputs)
    }
}

/// Unknown object field validation policy for schema-backed host contracts.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum UnknownFieldValidation {
    /// Allow object fields that are not declared by the registered schema.
    #[default]
    Allow,
    /// Reject object fields that are not declared by the registered schema.
    Reject,
}

impl UnknownFieldValidation {
    pub(crate) fn rejects_unknown_fields(self) -> bool {
        matches!(self, Self::Reject)
    }
}

/// The optional JavaScript built-ins each script context gets, chosen with
/// [`EngineOptions::builtins`].
///
/// Every context has ECMAScript's core (`Object`, `Array`, `String`, `Number`, `Math`,
/// `Symbol`, `BigInt`, `Reflect`, errors, ...), module support, `JSON`, `Map` and
/// `Set`, and `Promise`: Manivelle itself needs them. The others cost time and memory in
/// every context, which adds up for a game with hundreds of scripts: about half of the
/// time to create a context and a third of its memory. A script that uses a built-in
/// that is off fails with a `ReferenceError` when it runs; there is no static check, so
/// run your scripts under the options you ship.
///
/// The default enables everything, as a plain QuickJS context does.
///
/// New built-in flags may be added in minor releases, so build values from a constant
/// with struct update syntax, such as `ScriptBuiltins { date: true, ..ScriptBuiltins::NONE }`,
/// rather than listing every field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ScriptBuiltins {
    /// `RegExp`, regular expression literals, and the string methods that take them.
    pub regexp: bool,
    /// `Date`.
    pub date: bool,
    /// `Proxy`.
    pub proxy: bool,
    /// `ArrayBuffer`, the typed arrays and `DataView`. Keep it on when host functions
    /// use [`NativeBytes`](crate::NativeBytes), which crosses as a `Uint8Array`.
    pub typed_arrays: bool,
    /// `WeakRef` and `FinalizationRegistry`.
    pub weak_ref: bool,
    /// `atob`, `btoa`, `performance.now()`, `structuredClone`, and `TextEncoder` and
    /// `TextDecoder` (which also need `typed_arrays`).
    pub web: bool,
}

impl ScriptBuiltins {
    /// Every optional built-in: the default.
    pub const ALL: Self = Self::all(true);
    /// None of the optional built-ins: the cheapest context.
    pub const NONE: Self = Self::all(false);

    const fn all(enabled: bool) -> Self {
        Self {
            regexp: enabled,
            date: enabled,
            proxy: enabled,
            typed_arrays: enabled,
            weak_ref: enabled,
            web: enabled,
        }
    }
}

impl Default for ScriptBuiltins {
    fn default() -> Self {
        Self::ALL
    }
}

/// Configuration for [`crate::Engine`].
///
/// New fields may be added in minor releases, so build options with struct update
/// syntax, such as `EngineOptions { execution_timeout, ..EngineOptions::default() }`, rather
/// than listing every field.
#[derive(Debug, Clone)]
pub struct EngineOptions {
    /// Directory for transpiled JavaScript artifacts, reused across runs and reloads.
    /// `None` (the default) transpiles in memory on every load.
    pub cache_dir: Option<PathBuf>,
    /// Maximum wall time per load, call, emit, request or timer advance, including the
    /// Promise jobs it queues. Rust host handlers must return cooperatively; they
    /// cannot be preempted.
    pub execution_timeout: Duration,
    /// QuickJS memory limit in bytes.
    pub memory_limit_bytes: usize,
    /// QuickJS stack limit in bytes.
    pub max_stack_size_bytes: usize,
    /// Host contract validation policy.
    pub contract_validation: ContractValidation,
    /// Unknown object field validation policy for schema-backed host contracts.
    pub unknown_field_validation: UnknownFieldValidation,
    /// Optional JavaScript built-ins of every script context.
    pub builtins: ScriptBuiltins,
}

impl Default for EngineOptions {
    fn default() -> Self {
        Self {
            cache_dir: None,
            execution_timeout: Duration::from_secs(5),
            memory_limit_bytes: 16 * 1024 * 1024,
            max_stack_size_bytes: 512 * 1024,
            contract_validation: ContractValidation::Disabled,
            unknown_field_validation: UnknownFieldValidation::Allow,
            builtins: ScriptBuiltins::ALL,
        }
    }
}
