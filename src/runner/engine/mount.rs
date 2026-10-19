//! Mounting a script's module graph into its context, and swapping the new version in.

use std::cell::{Cell, RefCell};
use std::sync::Arc;

use rquickjs::{CatchResultExt, Context, Ctx, Module, Object, Persistent};

use crate::error::EngineError;
use crate::types::ScriptId;

use super::super::errors::{caught_js_error, js_error};
use super::super::host_promises::HostPromises;
use super::super::module_loader::{ModuleStore, RuntimeModuleGraph};
use super::super::quota::{Owner, Quota};
use super::super::retained::Retained;
use super::context::new_context;
use super::hot_reload::hot_hook_error;
use super::{
    Disposed, Engine, EngineScript, HotData, Prelude, ScriptGroup, ScriptHooks, ScriptOrigin,
    ScriptSignals,
};

/// The context a script mounts into, with the prelude's functions to build its
/// environment.
struct Target {
    context: Context,
    prelude: Prelude,
}

/// What mounting a script produced.
struct Mounted {
    context: Context,
    exports: Retained,
    signals: ScriptSignals,
    host_promises: HostPromises,
    hooks: ScriptHooks,
    owner: Option<Owner>,
}

/// What a script is being mounted with: who it is, its module graph, what its context
/// reports to the engine, and whether its context is its own.
pub(super) struct Mounting<'a> {
    pub(super) script_id: &'a str,
    graph: &'a RuntimeModuleGraph,
    pub(super) signals: &'a ScriptSignals,
    pub(super) host_promises: &'a HostPromises,
    own_context: bool,
}

impl Engine {
    /// Evaluates a freshly inserted graph, seeded with the `ctx.hot` state the loaded
    /// version saves, and swaps it in; on failure, the graph is removed and the
    /// previous version of the script stays loaded, not disposed.
    pub(super) fn mount_graph(
        &mut self,
        id: ScriptId,
        graph: RuntimeModuleGraph,
        origin: ScriptOrigin,
        group: Option<&str>,
    ) -> Result<Disposed, EngineError> {
        let mounted = self
            .save_hot_data(&id)
            .and_then(|hot_data| self.mount(&id, &graph, hot_data, group));
        match mounted {
            Ok(mounted) => {
                if let Some(group) = group.and_then(|name| self.groups.get_mut(name)) {
                    group.scripts += 1;
                }
                self.replace_script(
                    id,
                    EngineScript {
                        context: mounted.context,
                        group: group.map(Box::from),
                        graph_id: graph.graph_id,
                        hooks: mounted.hooks,
                        tasks: RefCell::new(Vec::new()),
                        request_guards: RefCell::new(Vec::new()),
                        exports: mounted.exports,
                        signals: mounted.signals,
                        host_promises: mounted.host_promises,
                        module_ids: graph.module_ids,
                        origin,
                        faults: Cell::new(0),
                        owner: mounted.owner,
                        quarantined: Cell::new(false),
                    },
                )
            }
            Err(error) => {
                self.module_store.remove_modules(&graph.module_ids)?;
                if let Some(project) = origin.project {
                    self.watch_failed_attempt(&id, project.watched);
                }
                Err(error)
            }
        }
    }

    /// Builds the script's environment in its context (a group's, created by its first
    /// script) and imports its entry module.
    fn mount(
        &mut self,
        script_id: &str,
        graph: &RuntimeModuleGraph,
        hot_data: Option<HotData>,
        group: Option<&str>,
    ) -> Result<Mounted, EngineError> {
        // The guard borrows a handle of its own, not `self`, which the group lookup
        // below borrows mutably.
        let execution = Arc::clone(&self.execution);
        let _budget = execution.enter(self.execution_timeout);
        // Everything the mount allocates, the context included, is the script's. Held
        // through a handle of its own: `self` is borrowed mutably below.
        let quota = self.faults.quota.clone();
        let owner = quota.as_ref().map(Quota::new_owner);
        let _charge = quota
            .as_deref()
            .zip(owner.as_ref())
            .map(|(q, o)| q.charge_to(o));
        // Read before `target` inserts the group, so a group this mount created is known.
        let created_group = group.filter(|name| !self.groups.contains_key(*name));
        let target = match self.target(group) {
            Ok(target) => target,
            Err(error) => return self.attribute_interrupt(self.settle(Err(error))),
        };
        if group.is_some() {
            self.module_store.mark_grouped(graph.graph_id)?;
        }
        let signals = ScriptSignals::new(&self.event_index);
        let host_promises = HostPromises::with_wake(Arc::clone(&self.scheduling.host_wake));
        let mounting = Mounting {
            script_id,
            graph,
            signals: &signals,
            host_promises: &host_promises,
            own_context: group.is_none(),
        };
        let mounted = target
            .context
            .with(|ctx| self.mount_in(&ctx, &target.prelude, &mounting, hot_data));
        match self.attribute_interrupt(self.settle(mounted)) {
            Ok((exports, hooks)) => Ok(Mounted {
                context: target.context,
                exports,
                signals,
                host_promises,
                hooks,
                owner,
            }),
            Err(error) => {
                self.roll_back_mount(graph, created_group);
                Err(error)
            }
        }
    }

    /// The part of mounting that runs JavaScript, in the script's context: builds its
    /// environment, exposes it, and imports the entry module.
    fn mount_in<'js>(
        &self,
        ctx: &Ctx<'js>,
        prelude: &Prelude,
        mounting: &Mounting<'_>,
        hot_data: Option<HotData>,
    ) -> Result<(Retained, ScriptHooks), EngineError> {
        let (env, hooks) = self.build_env(ctx, prelude, mounting, hot_data)?;
        self.expose_env(ctx, prelude, mounting, env)?;
        let exports = import_exports(ctx, &self.module_store, &mounting.graph.entry_module_id)?;
        Ok((exports, hooks))
    }

    /// Makes the script's environment reachable by the script: as globals in a context of
    /// its own, or through its `manivelle:env` module in a group's, whose context has no
    /// per-script globals.
    fn expose_env<'js>(
        &self,
        ctx: &Ctx<'js>,
        prelude: &Prelude,
        mounting: &Mounting<'_>,
        env: Object<'js>,
    ) -> Result<(), EngineError> {
        if !mounting.own_context {
            self.envs
                .insert(mounting.graph.graph_id, Persistent::save(ctx, env));
            return Ok(());
        }
        prelude
            .install_globals
            .clone()
            .restore(ctx)
            .map_err(js_error)?
            .call::<_, ()>((env,))
            .catch(ctx)
            .map_err(caught_js_error)
    }

    /// Undoes what a failed mount left: the script's environment, and the group it created.
    fn roll_back_mount(&mut self, graph: &RuntimeModuleGraph, created_group: Option<&str>) {
        self.envs.remove(graph.graph_id);
        if let Some(name) = created_group {
            self.groups.remove(name);
        }
    }

    /// The context a script mounts into: a new one of its own, or its group's, created
    /// the first time the group is used.
    fn target(&mut self, group: Option<&str>) -> Result<Target, EngineError> {
        if let Some(existing) = group.and_then(|name| self.groups.get(name)) {
            return Ok(Target {
                context: existing.context.clone(),
                prelude: existing.prelude.clone(),
            });
        }
        let context = new_context(&self.runtime, self.builtins)?;
        let prelude = context.with(|ctx| self.run_prelude(&ctx))?;
        if let Some(name) = group {
            self.groups.insert(
                Box::from(name),
                ScriptGroup {
                    context: context.clone(),
                    prelude: prelude.clone(),
                    scripts: 0,
                },
            );
        }
        Ok(Target { context, prelude })
    }

    /// Swaps `script` in, then retires the version it replaces, whose failed `dispose`
    /// does not undo the swap.
    fn replace_script(
        &mut self,
        id: ScriptId,
        script: EngineScript,
    ) -> Result<Disposed, EngineError> {
        let previous = self.scripts.insert(id.clone(), script);
        self.event_index.mark_stale();
        let Some(previous) = previous else {
            return Ok(Ok(()));
        };
        Ok(self.retire(previous)?.map_err(|error| {
            hot_hook_error(
                error,
                &format!("`ctx.hot.dispose` of the previous version of script `{id}` failed; the new version is loaded"),
            )
        }))
    }
}

/// Imports a script's entry module, which only the engine may import from outside the
/// script's graph, and returns its namespace.
fn import_exports(
    ctx: &Ctx<'_>,
    module_store: &ModuleStore,
    entry_module_id: &str,
) -> Result<Retained, EngineError> {
    let exports = module_store
        .host_import(entry_module_id, || Module::import(ctx, entry_module_id))?
        .and_then(|promise| promise.finish::<Object<'_>>())
        .catch(ctx)
        .map_err(caught_js_error)?;
    Ok(Retained::new(exports.as_value()))
}
