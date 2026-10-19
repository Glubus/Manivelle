//! Identity of the script calling a host function.

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
}

impl<'a> Caller<'a> {
    pub(crate) fn new(script_id: &'a str) -> Self {
        Self { script_id }
    }

    /// Id the calling script was loaded under, the one passed to
    /// [`Engine::load_script`](crate::Engine::load_script) or
    /// [`Engine::load_project`](crate::Engine::load_project). A reloaded script keeps
    /// its id.
    pub fn script_id(&self) -> &'a str {
        self.script_id
    }
}
