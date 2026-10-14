//! QuickJS loader and resolver adapters backed by the module store.

use rquickjs::loader::{ImportAttributes, Loader, Resolver};
use rquickjs::{Ctx, Error, Module, Result as JsResult};

use super::envs::EnvHandle;
use super::graph::ENV_MODULE_PREFIX;
use super::store::ModuleStore;

/// The module a context-group script imports as `manivelle:env`: the script's environment
/// object, which the loader sets as `import.meta.env`. Only this module sees it, so one
/// script cannot reach another's.
const ENV_MODULE_SOURCE: &str = r#"
const env = import.meta.env;
export const ctx = env.ctx;
export const console = env.console;
export const setTimeout = env.setTimeout;
export const setInterval = env.setInterval;
export const clearTimeout = env.clearTimeout;
export const clearInterval = env.clearInterval;
export const __native = env.native;
export const __on = env.on;
"#;

#[derive(Debug, Clone)]
pub(crate) struct MemoryModuleResolver {
    store: ModuleStore,
}

impl MemoryModuleResolver {
    pub(crate) fn new(store: ModuleStore) -> Self {
        Self { store }
    }
}

impl Resolver for MemoryModuleResolver {
    fn resolve<'js>(
        &mut self,
        _ctx: &Ctx<'js>,
        base: &str,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> JsResult<String> {
        self.store.resolve(base, name)
    }
}

pub(crate) struct MemoryModuleLoader {
    store: ModuleStore,
    envs: EnvHandle,
}

impl MemoryModuleLoader {
    pub(crate) fn new(store: ModuleStore, envs: EnvHandle) -> Self {
        Self { store, envs }
    }
}

impl Loader for MemoryModuleLoader {
    fn load<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        name: &str,
        _attributes: Option<ImportAttributes<'js>>,
    ) -> JsResult<Module<'js>> {
        let Some(graph_id) = name
            .strip_prefix(ENV_MODULE_PREFIX)
            .and_then(|id| id.parse().ok())
        else {
            return self.store.load(ctx, name);
        };
        let env = self
            .envs
            .env(ctx, graph_id)
            .ok_or_else(|| Error::new_loading_message(name, "the script's environment is gone"))?;
        let module = Module::declare(ctx.clone(), name, ENV_MODULE_SOURCE)?;
        module.meta()?.set("env", env)?;
        Ok(module)
    }
}
