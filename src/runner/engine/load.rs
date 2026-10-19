//! Loading scripts from source or from a project, into a context of their own or into a
//! group's.

use std::path::Path;
use std::rc::Rc;

use crate::compiler::ProjectFailure;
use crate::error::EngineError;
use crate::packages::PermissionSet;
use crate::registry::HostModuleStyle;
use crate::types::ScriptId;

use super::super::module_loader::ENV_SPECIFIER;
use super::prelude::GLOBALS_ENV_MODULE;
use super::{Disposed, Engine, ProjectFiles, ScriptOrigin};

impl Engine {
    /// Loads or replaces one TypeScript script. It may import host modules, not other
    /// files; use [`Engine::load_project`] for those. A failed load keeps the previous
    /// version; see [`Engine`] for the `ctx.hot` state a replaced version hands over.
    pub fn load_script(
        &mut self,
        id: impl Into<ScriptId>,
        source: &str,
    ) -> Result<(), EngineError> {
        self.load_forgetting_permissions(id.into(), |engine, id| {
            engine.mount_inline(None, id, source)
        })
    }

    /// [`Engine::load_script`] into context group `group`: the scripts loaded under one
    /// group name share a QuickJS context, which makes them cheaper to load, smaller and,
    /// from a few hundred scripts on, faster to deliver events to. Each keeps its own
    /// events, timers, `console`, host-function identity ([`Caller`](crate::Caller)),
    /// `ctx.hot` state and lifecycle, and reaches them with
    /// `import { ctx, console, setTimeout } from "manivelle:env"`: a shared context has no
    /// per-script globals, so a script of a group has none of `ctx`, `console`, the timers
    /// or the namespaced host globals as globals, and reaches host functions through
    /// host module imports.
    ///
    /// A group is a trust boundary: its scripts share the built-ins and the global
    /// object, so one can patch `Array.prototype` or set `globalThis.x` for the others.
    /// Put scripts of one author in one group, and mods of different authors in
    /// different groups, or load them with [`Engine::load_script`]. Reloading a script of
    /// a group leaves its previous modules in the group's context until the context
    /// drops, so a development loop that reloads one script many times grows memory
    /// slowly. Unloading a group's last script drops its context.
    pub fn load_script_in(
        &mut self,
        group: &str,
        id: impl Into<ScriptId>,
        source: &str,
    ) -> Result<(), EngineError> {
        self.load_forgetting_permissions(id.into(), |engine, id| {
            engine.mount_inline(Some(group), id, source)
        })
    }

    /// Runs one load's mount, then drops the permissions a package load gave `id`. A load
    /// that fails before the new version is swapped in keeps them.
    fn load_forgetting_permissions(
        &mut self,
        id: ScriptId,
        mount: impl FnOnce(&mut Self, ScriptId) -> Result<Disposed, EngineError>,
    ) -> Result<(), EngineError> {
        let loaded = mount(self, id.clone())?;
        self.permissions.remove(&id);
        loaded
    }

    fn mount_inline(
        &mut self,
        group: Option<&str>,
        id: ScriptId,
        source: &str,
    ) -> Result<Disposed, EngineError> {
        let script = self.transpiler.inline(&id, source)?;
        self.install_host_modules()?;
        let graph_id = self.next_graph_id();
        let graph =
            self.module_store
                .insert_inline(&id, script.js, script.module_origin, graph_id)?;
        let origin = ScriptOrigin {
            module_keys: vec![script.module_key],
            project: None,
        };
        self.mount_graph(id, graph, origin, group)
    }

    /// Loads or replaces one multi-file TypeScript project from its entry file.
    ///
    /// The static ESM graph is resolved from disk: relative imports, `tsconfig.json`
    /// `paths` and `baseUrl`, and packages in the project's `node_modules`. Dynamic
    /// `import()` is rejected. Modules are transpiled one by one and remembered by
    /// content, so reloading a project only transpiles the files that changed. A
    /// failed load keeps the previous version; see [`Engine`] for the `ctx.hot` state
    /// a replaced version hands over.
    pub fn load_project(
        &mut self,
        id: impl Into<ScriptId>,
        entry_path: impl AsRef<Path>,
    ) -> Result<(), EngineError> {
        self.load_forgetting_permissions(id.into(), |engine, id| {
            engine.mount_project(None, id, entry_path.as_ref())
        })
    }

    /// [`Engine::load_project`] into context group `group`; see
    /// [`Engine::load_script_in`] for what a group shares and what it keeps apart.
    pub fn load_project_in(
        &mut self,
        group: &str,
        id: impl Into<ScriptId>,
        entry_path: impl AsRef<Path>,
    ) -> Result<(), EngineError> {
        self.load_forgetting_permissions(id.into(), |engine, id| {
            engine.mount_project(Some(group), id, entry_path.as_ref())
        })
    }

    /// [`Engine::load_project`] for a package: its script reaches only the host
    /// contracts `permissions` allows. A failed load keeps the previous version and its
    /// permissions.
    pub(crate) fn load_project_restricted(
        &mut self,
        id: impl Into<ScriptId>,
        entry_path: impl AsRef<Path>,
        permissions: PermissionSet,
    ) -> Result<(), EngineError> {
        let id = id.into();
        let previous = self.permissions.insert(id.clone(), Rc::new(permissions));
        let loaded = self.mount_project(None, id.clone(), entry_path.as_ref());
        if loaded.is_err() {
            match previous {
                Some(previous) => self.permissions.insert(id, previous),
                None => self.permissions.remove(&id),
            };
        }
        loaded?
    }

    /// [`Engine::load_project`], keeping a failed load apart from a failed `dispose` of
    /// the version it replaced.
    pub(super) fn mount_project(
        &mut self,
        group: Option<&str>,
        id: ScriptId,
        entry_path: &Path,
    ) -> Result<Disposed, EngineError> {
        let mut external_modules = self.registry.import_module_names()?;
        external_modules.insert(ENV_SPECIFIER.to_owned());
        let project = match self.transpiler.project(entry_path, &external_modules) {
            Ok(project) => project,
            Err(ProjectFailure { error, watched }) => {
                self.watch_failed_attempt(&id, watched);
                return Err(error);
            }
        };
        self.install_host_modules()?;
        let graph_id = self.next_graph_id();
        let graph = self.module_store.insert_project(
            &project.entry_module_id,
            project.modules,
            graph_id,
        )?;
        let origin = ScriptOrigin {
            module_keys: project.module_keys,
            project: Some(ProjectFiles {
                entry_path: entry_path.to_path_buf(),
                watched: project.watched,
            }),
        };
        self.mount_graph(id, graph, origin, group)
    }

    fn install_host_modules(&self) -> Result<(), EngineError> {
        let mut modules = self.registry.import_modules(HostModuleStyle::Globals)?;
        modules.insert(ENV_SPECIFIER.to_owned(), GLOBALS_ENV_MODULE.to_owned());
        self.module_store.insert_host_modules(modules)?;
        self.module_store
            .insert_group_host_modules(self.registry.import_modules(HostModuleStyle::Env)?)
    }

    fn next_graph_id(&mut self) -> u64 {
        let graph_id = self.next_graph_id;
        self.next_graph_id += 1;
        graph_id
    }
}
