//! In-memory module source and resolution store.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};

use rquickjs::{Ctx, Error, Module, Result as JsResult};

use crate::compiler::{CompiledModule, ModuleOrigin};
use crate::error::EngineError;

use super::graph::RuntimeModuleGraph;

mod inner;
mod insertion;
mod removal;
mod resolution;

#[cfg(test)]
mod tests;

use inner::ModuleStoreInner;

#[derive(Debug, Clone, Default)]
pub(crate) struct ModuleStore {
    inner: Arc<Mutex<ModuleStoreInner>>,
}

impl ModuleStore {
    pub(crate) fn insert_host_modules(
        &self,
        modules: BTreeMap<String, String>,
    ) -> std::result::Result<(), EngineError> {
        let mut guard = self.lock_store()?;
        insertion::insert_host_modules(&mut guard, modules);
        Ok(())
    }

    /// Sources of the host modules as a context group's scripts import them.
    pub(crate) fn insert_group_host_modules(
        &self,
        modules: BTreeMap<String, String>,
    ) -> std::result::Result<(), EngineError> {
        let mut guard = self.lock_store()?;
        insertion::insert_group_host_modules(&mut guard, modules);
        Ok(())
    }

    /// Marks graph `graph_id` as loaded into a context group; removing its modules
    /// clears the mark.
    pub(crate) fn mark_grouped(&self, graph_id: u64) -> std::result::Result<(), EngineError> {
        self.lock_store()?.grouped_graphs.insert(graph_id);
        Ok(())
    }

    pub(crate) fn insert_inline(
        &self,
        script_id: &str,
        source: String,
        origin: ModuleOrigin,
        graph_id: u64,
    ) -> std::result::Result<RuntimeModuleGraph, EngineError> {
        let mut guard = self.lock_store()?;
        Ok(insertion::insert_inline(
            &mut guard, script_id, source, origin, graph_id,
        ))
    }

    pub(crate) fn insert_project(
        &self,
        entry_module_id: &str,
        modules: Vec<CompiledModule>,
        graph_id: u64,
    ) -> std::result::Result<RuntimeModuleGraph, EngineError> {
        let mut guard = self.lock_store()?;
        insertion::insert_project(&mut guard, entry_module_id, modules, graph_id)
    }

    pub(crate) fn remove_modules(
        &self,
        module_ids: &[String],
    ) -> std::result::Result<(), EngineError> {
        let mut guard = self.lock_store()?;
        removal::remove_modules(&mut guard, module_ids);
        Ok(())
    }

    pub(super) fn resolve(&self, base: &str, name: &str) -> JsResult<String> {
        let mut guard = self
            .inner
            .lock()
            .map_err(|_| Error::new_resolving_message(base, name, "module store lock poisoned"))?;
        resolution::resolve_from_store(&mut guard, base, name)
    }

    /// Lets the engine import a script's entry module, `module_id`, from outside its
    /// graph, once, while `import` runs: a script module is otherwise only reachable
    /// from its own graph.
    pub(crate) fn host_import<T>(
        &self,
        module_id: &str,
        import: impl FnOnce() -> T,
    ) -> std::result::Result<T, EngineError> {
        self.lock_store()?.host_import = Some(module_id.to_owned());
        let imported = import();
        self.lock_store()?.host_import = None;
        Ok(imported)
    }

    /// `path:line:column` of the TypeScript behind a 1-based position in a loaded
    /// script module; `None` for host modules and unmapped positions.
    pub(crate) fn locate(&self, module_id: &str, line: u32, column: u32) -> Option<String> {
        let guard = self.inner.lock().ok()?;
        guard.origins.get(module_id)?.locate(line, column)
    }

    pub(super) fn load<'js>(&self, ctx: &Ctx<'js>, name: &str) -> JsResult<Module<'js>> {
        let source = {
            let guard = self
                .inner
                .lock()
                .map_err(|_| Error::new_loading_message(name, "module store lock poisoned"))?;
            resolution::source_for_load(&guard, name)?
        };

        Module::declare(ctx.clone(), name, source)
    }

    fn lock_store(
        &self,
    ) -> std::result::Result<std::sync::MutexGuard<'_, ModuleStoreInner>, EngineError> {
        self.inner.lock().map_err(|_| EngineError::LockPoisoned)
    }
}
