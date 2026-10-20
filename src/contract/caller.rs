//! Identity of the script calling a host function, and the host state it may reach.

use std::any::Any;

use super::state::StateSlot;
use crate::error::EngineError;

/// The script a host function call comes from.
///
/// Handlers registered with
/// [`InMemoryHostContractRegistry::function_with_caller`](crate::InMemoryHostContractRegistry::function_with_caller)
/// or
/// [`InMemoryHostContractRegistry::async_function_with_caller`](crate::InMemoryHostContractRegistry::async_function_with_caller)
/// receive it with every call.
#[derive(Debug, Clone, Copy)]
pub struct Caller<'a> {
    script_id: &'a str,
    state: &'a StateSlot,
}

impl<'a> Caller<'a> {
    pub(crate) fn new(script_id: &'a str, state: &'a StateSlot) -> Self {
        Self { script_id, state }
    }

    /// Id the calling script was loaded under, the one passed to
    /// [`Engine::load_script`](crate::Engine::load_script) or
    /// [`Engine::load_project`](crate::Engine::load_project). A reloaded script keeps
    /// its id.
    pub fn script_id(&self) -> &'a str {
        self.script_id
    }

    /// Runs `f` with the state the host lent through
    /// [`Engine::with_state`](crate::Engine::with_state), as a `&mut T`.
    ///
    /// The state is lent for `f` only, so two borrows cannot overlap: a host function that
    /// calls back into the engine, and a script that reaches this function again, gets
    /// [`EngineError::StateBusy`] instead of a second `&mut`.
    ///
    /// # Errors
    ///
    /// - [`EngineError::NoState`]: the call did not run inside `with_state`, for example
    ///   a timer the host advanced outside a scope.
    /// - [`EngineError::StateType`]: the scope lent a type other than `T`.
    /// - [`EngineError::StateBusy`]: a host function up the call stack holds the state.
    pub fn with_state<T: Any, R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, EngineError> {
        self.state.with(f)
    }
}
