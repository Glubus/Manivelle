//! Stopping an engine's running JavaScript from another thread.

use std::sync::Arc;

use super::execution::ExecutionControl;

/// Stops the JavaScript an [`Engine`](crate::Engine) is running, from any thread.
///
/// Get one with [`Engine::interrupt_handle`](crate::Engine::interrupt_handle). The
/// engine itself stays on its thread; the handle only shares an atomic flag.
///
/// [`interrupt`](Self::interrupt) stops the load, call, emit, request or timer advance
/// in progress at its next QuickJS interrupt check; that operation fails with
/// [`EngineError::Interrupted`](crate::EngineError::Interrupted) and the engine stays usable.
/// When no operation is running, the request has no effect: the next operation
/// starts normally. Like the execution timeout, it cannot stop a Rust host function
/// that blocks; the script stops once the host function returns.
#[derive(Debug, Clone)]
pub struct InterruptHandle {
    control: Arc<ExecutionControl>,
}

impl InterruptHandle {
    pub(crate) fn new(control: Arc<ExecutionControl>) -> Self {
        Self { control }
    }

    /// Asks the engine to stop the JavaScript it is running.
    pub fn interrupt(&self) {
        self.control.request_interrupt();
    }
}
