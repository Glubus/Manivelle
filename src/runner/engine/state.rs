//! Lending a host state to the host functions that run during a scope.

use std::any::Any;

use super::Engine;
use crate::error::EngineError;

impl Engine {
    /// Lends `state` to the host functions that run while `f` does, and returns what `f`
    /// returns.
    ///
    /// `f` gets the engine back: every call that can enter a script (`call`, `emit`,
    /// `request`, `advance_timers`, `pump`, `load_script`, `reload_changed`, ...) can run
    /// inside it, and the host functions it reaches read the state with
    /// [`Caller::with_state`](crate::Caller::with_state). Outside a scope no state is
    /// lent, and a host function that asks gets [`EngineError::NoState`].
    ///
    /// `state` stays exclusively borrowed until `with_state` returns, so the host cannot
    /// touch it while scripts run, and a host function cannot keep it afterwards. The
    /// state is taken back when `f` returns or panics.
    ///
    /// # Errors
    ///
    /// [`EngineError::StateScopeActive`] when called from inside another scope: states do
    /// not nest.
    pub fn with_state<S: Any, R>(
        &mut self,
        state: &mut S,
        f: impl FnOnce(&mut Engine) -> R,
    ) -> Result<R, EngineError> {
        let _scope = self.state.enter(state)?;
        Ok(f(self))
    }
}
