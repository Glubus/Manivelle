//! Loading, reloading and unloading scripts, and the `ctx.hot` hand-over between
//! versions.

use std::cell::{Cell, RefCell};
use std::path::Path;
use std::rc::Rc;
use std::sync::Arc;

use rquickjs::module::{Declared, Evaluated};
use rquickjs::{
    CatchResultExt, Context, Ctx, Function, Module, Object, Persistent, Runtime, Value as JsValue,
    WriteOptions, qjs,
};

use crate::compiler::{ProjectFailure, WatchedFiles};
use crate::config::ScriptBuiltins;
use crate::error::EngineError;
use crate::packages::PermissionSet;
use crate::registry::{HostModuleStyle, NativeContext};
use crate::types::{ReloadReport, ScriptId};

use super::super::errors::{caught_js_error, js_error};
use super::super::host_fn::HostFnClass;
use super::super::host_promises::HostPromises;
use super::super::module_loader::{ENV_SPECIFIER, ModuleStore, RuntimeModuleGraph};
use super::super::quota::{Owner, Quota};
use super::super::retained::Retained;
use super::super::timers::timer_hooks;
use super::super::web;
use super::{
    Disposed, Engine, EngineScript, HotData, Prelude, ProjectFiles, ScriptGroup, ScriptHooks,
    ScriptOrigin, ScriptSignals, script_not_found,
};

/// Builds `ctx.on` / `ctx.off` and their handler lists, `ctx.hot`, `console` and the timer
/// functions of one script, and can expose them, with `__host` and the namespaced host
/// globals, as globals. A module, so that its compiled form can be kept and loaded into
/// every context.
const CONTEXT_PRELUDE: &str = include_str!("../../../assets/context_prelude.js");
const PRELUDE_MODULE: &str = "manivelle:prelude";

/// What `import ... from "manivelle:env"` reads in a context of its own: the globals the
/// prelude exposed. A script in a group gets its own environment module instead.
const GLOBALS_ENV_MODULE: &str = r#"
export const ctx = globalThis.ctx;
export const console = globalThis.console;
export const setTimeout = globalThis.setTimeout;
export const setInterval = globalThis.setInterval;
export const clearTimeout = globalThis.clearTimeout;
export const clearInterval = globalThis.clearInterval;
"#;

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
struct Mounting<'a> {
    script_id: &'a str,
    graph: &'a RuntimeModuleGraph,
    signals: &'a ScriptSignals,
    host_promises: &'a HostPromises,
    own_context: bool,
}

impl Engine {
    /// Loads or replaces one TypeScript script. It may import host modules, not other
    /// files; use [`Engine::load_project`] for those. A failed load keeps the previous
    /// version; see [`Engine`] for the `ctx.hot` state a replaced version hands over.
    pub fn load_script(
        &mut self,
        id: impl Into<ScriptId>,
        source: &str,
    ) -> Result<(), EngineError> {
        let id = id.into();
        let loaded = self.mount_inline(None, id.clone(), source)?;
        self.permissions.remove(&id);
        loaded
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
        let id = id.into();
        let loaded = self.mount_inline(Some(group), id.clone(), source)?;
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
        let id = id.into();
        let loaded = self.mount_project(None, id.clone(), entry_path.as_ref())?;
        self.permissions.remove(&id);
        loaded
    }

    /// [`Engine::load_project`] into context group `group`; see
    /// [`Engine::load_script_in`] for what a group shares and what it keeps apart.
    pub fn load_project_in(
        &mut self,
        group: &str,
        id: impl Into<ScriptId>,
        entry_path: impl AsRef<Path>,
    ) -> Result<(), EngineError> {
        let id = id.into();
        let loaded = self.mount_project(Some(group), id.clone(), entry_path.as_ref())?;
        self.permissions.remove(&id);
        loaded
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
    fn mount_project(
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

    /// Reloads, in load order, every project whose files changed since it was loaded,
    /// and reports which reloads succeeded and which failed. A reload whose replaced
    /// version's `ctx.hot.dispose` threw is listed both in `reloaded` and in
    /// `dispose_failed`.
    ///
    /// A project is checked through the size and modification time of its module
    /// files, their directories, its `tsconfig.json` and the configs that one
    /// `extends` by path, and the `package.json` files its imports resolved through;
    /// nothing is read unless one of them changed. Inline scripts are never reloaded
    /// here. A failed reload keeps the previous version running and is reported once:
    /// the next report only lists it again after another change, to the previous
    /// version's files or to those the failed reload had reached (a module it newly
    /// imported, say). Call it from the host loop, for example once per second during
    /// development; it starts no thread.
    pub fn reload_changed(&mut self) -> ReloadReport {
        let changed = self
            .scripts
            .iter()
            .filter_map(|(id, script)| {
                let project = script.origin.project.as_ref()?;
                project
                    .watched
                    .changed()
                    .then(|| (id.clone(), project.entry_path.clone(), script.group.clone()))
            })
            .collect::<Vec<_>>();

        let mut report = ReloadReport::default();
        for (id, entry_path, group) in changed {
            match self.mount_project(group.as_deref(), id.clone(), &entry_path) {
                Ok(disposed) => {
                    if let Err(error) = disposed {
                        report.dispose_failed.push((id.clone(), error));
                    }
                    report.reloaded.push(id);
                }
                Err(error) => {
                    self.restamp(&id);
                    report.failed.push((id, error));
                }
            }
        }
        report
    }

    /// Takes a project's files as they are now as seen, so a failed reload is reported
    /// once.
    fn restamp(&mut self, id: &str) {
        if let Some(project) = self
            .scripts
            .get_mut(id)
            .and_then(|script| script.origin.project.as_mut())
        {
            project.watched.restamp();
        }
    }

    /// Adds the files a failed load of project `id` looked at to the files of its
    /// loaded version: an edit to one of them, such as a module the failed version
    /// imported first, may fix the load, so [`Engine::reload_changed`] must see it.
    fn watch_failed_attempt(&mut self, id: &str, watched: WatchedFiles) {
        if let Some(project) = self
            .scripts
            .get_mut(id)
            .and_then(|script| script.origin.project.as_mut())
        {
            project.watched.merge(watched);
        }
    }

    /// Unloads one script and releases its modules. Its `ctx.hot.dispose` callbacks run
    /// first; when one throws, the script is unloaded all the same and the error is
    /// returned.
    pub fn unload_script(&mut self, id: &str) -> Result<(), EngineError> {
        let script = self
            .scripts
            .shift_remove(id)
            .ok_or_else(|| script_not_found(id))?;
        self.permissions.remove(id);
        self.event_index.mark_stale();
        self.retire(script)?.map_err(|error| {
            hot_hook_error(
                error,
                &format!("`ctx.hot.dispose` of script `{id}` failed; the script is unloaded"),
            )
        })
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

    /// Evaluates a freshly inserted graph, seeded with the `ctx.hot` state the loaded
    /// version saves, and swaps it in; on failure, the graph is removed and the
    /// previous version of the script stays loaded, not disposed.
    fn mount_graph(
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

    /// Runs the loaded version's `ctx.hot.save` callback, when the script is loaded;
    /// its result becomes the next version's `ctx.hot.data`.
    fn save_hot_data(&self, id: &str) -> Result<Option<HotData>, EngineError> {
        let Some(script) = self.scripts.get(id) else {
            return Ok(None);
        };
        let _budget = self.budget();
        let _charge = self.charge_to(script);
        let saved = script.context.with(|ctx| {
            call_hot_hook(&ctx, &script.hooks.hot_save).map(|data| Persistent::save(&ctx, data))
        });
        self.attribute_interrupt(self.settle(saved))
            .map(Some)
            .map_err(|error| {
                hot_hook_error(
                    error,
                    &format!(
                        "`ctx.hot.save` of script `{id}` failed; the previous version keeps running"
                    ),
                )
            })
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
        let quota = self.quota.clone();
        let owner = quota.as_ref().map(Quota::new_owner);
        let _charge = quota
            .as_deref()
            .zip(owner.as_ref())
            .map(|(q, o)| q.charge_to(o));
        let created_group = group.is_some_and(|name| !self.groups.contains_key(name));
        let target = match self.target(group) {
            Ok(target) => target,
            Err(error) => return self.attribute_interrupt(self.settle(Err(error))),
        };
        if group.is_some() {
            self.module_store.mark_grouped(graph.graph_id)?;
        }
        let signals = ScriptSignals::new(&self.event_index);
        let host_promises = HostPromises::with_wake(Arc::clone(&self.host_wake));
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
                self.roll_back_mount(graph, group.filter(|_| created_group));
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

    /// Runs the context prelude in `ctx` and declares the web APIs it can load.
    fn run_prelude(&self, ctx: &Ctx<'_>) -> Result<Prelude, EngineError> {
        let module = self.evaluate_prelude(ctx)?;
        if self.builtins.web {
            self.declare_web_apis(ctx, &module)?;
        }
        Ok(Prelude {
            make_env: export_function(ctx, &module, "makeEnv")?,
            install_globals: export_function(ctx, &module, "installGlobals")?,
        })
    }

    /// Evaluates the prelude module in `ctx`.
    fn evaluate_prelude<'js>(&self, ctx: &Ctx<'js>) -> Result<Module<'js, Evaluated>, EngineError> {
        let declared = self
            .declare_prelude(ctx)
            .catch(ctx)
            .map_err(caught_js_error)?;
        let (module, evaluated) = declared.eval().catch(ctx).map_err(caught_js_error)?;
        evaluated
            .finish::<()>()
            .catch(ctx)
            .map_err(caught_js_error)?;
        Ok(module)
    }

    /// The prelude module, declared from the bytecode the first mount kept, or compiled
    /// from the source when there is none yet: every later context loads the bytecode
    /// instead of parsing and compiling the source again, which was most of the cost of
    /// mounting a script.
    fn declare_prelude<'js>(&self, ctx: &Ctx<'js>) -> rquickjs::Result<Module<'js, Declared>> {
        if let Some(bytecode) = self.prelude.get() {
            // SAFETY: QuickJS does not verify bytecode. These bytes were written by this
            // process, from the crate's own prelude, by the same QuickJS build.
            return unsafe { Module::load(ctx.clone(), bytecode) };
        }
        let declared = Module::declare(ctx.clone(), PRELUDE_MODULE, CONTEXT_PRELUDE)?;
        if let Ok(bytecode) = declared.write(WriteOptions::default()) {
            // Only this thread sets it, once: the first mount.
            let _ = self.prelude.set(bytecode.into_boxed_slice());
        }
        Ok(declared)
    }

    /// Declares the web globals as lazy accessors that load their module on first use.
    fn declare_web_apis<'js>(
        &self,
        ctx: &Ctx<'js>,
        prelude: &Module<'js, Evaluated>,
    ) -> Result<(), EngineError> {
        prelude
            .get::<_, Function<'_>>("installWebApis")
            .map_err(js_error)?
            .call::<_, ()>((
                web::loader(ctx, &self.web_bytecode)?,
                self.web_global_names(),
            ))
            .catch(ctx)
            .map_err(caught_js_error)
    }

    /// The web globals this engine's built-ins provide: `TextEncoder` and `TextDecoder`
    /// need typed arrays.
    fn web_global_names(&self) -> Vec<&'static str> {
        let mut names = vec!["structuredClone"];
        if self.builtins.typed_arrays {
            names.extend(["TextEncoder", "TextDecoder"]);
        }
        names
    }

    /// Builds the environment of a script: its host functions, bound to its id, the hooks
    /// through which its handlers and timers report, its `console` sink and the state the
    /// previous version saved. Returns it with the functions the engine calls.
    fn build_env<'js>(
        &self,
        ctx: &Ctx<'js>,
        prelude: &Prelude,
        mounting: &Mounting<'_>,
        hot_data: Option<HotData>,
    ) -> Result<(Object<'js>, ScriptHooks), EngineError> {
        let hooks = self.env_hooks(ctx, mounting, hot_data)?;
        let env = prelude
            .make_env
            .clone()
            .restore(ctx)
            .map_err(js_error)?
            .call::<_, Object<'js>>((hooks,))
            .catch(ctx)
            .map_err(caught_js_error)?;
        let script_hooks = read_script_hooks(ctx, &env)?;
        Ok((env, script_hooks))
    }

    /// The object `makeEnv` of the prelude receives: what the engine bound to the script.
    fn env_hooks<'js>(
        &self,
        ctx: &Ctx<'js>,
        mounting: &Mounting<'_>,
        hot_data: Option<HotData>,
    ) -> Result<Object<'js>, EngineError> {
        let signals = mounting.signals;
        let timers = timer_hooks(
            ctx,
            &self.timer_clock,
            &signals.next_timer,
            &self.earliest_timer,
        )?;
        let hooks = Object::new(ctx.clone()).map_err(js_error)?;
        let set = |name: &str, value: JsValue<'js>| hooks.set(name, value).map_err(js_error);
        set("native", self.host_functions(ctx, mounting)?.into_value())?;
        let allowed = self.permissions.get(mounting.script_id).cloned();
        set(
            "handlers",
            signals
                .events
                .hook(ctx, mounting.script_id, allowed)?
                .into_value(),
        )?;
        set(
            "console",
            self.console
                .writer(ctx, mounting.script_id, &self.module_store)?
                .into_value(),
        )?;
        set("now", timers.now.into_value())?;
        set("schedule", timers.schedule.into_value())?;
        if let Some(hot_data) = hot_data {
            set("hotData", hot_data.restore(ctx).map_err(js_error)?)?;
        }
        Ok(hooks)
    }

    /// The script's host functions, by contract name, each bound to the script's id.
    fn host_functions<'js>(
        &self,
        ctx: &Ctx<'js>,
        mounting: &Mounting<'_>,
    ) -> Result<Object<'js>, EngineError> {
        let functions = Object::new(ctx.clone()).map_err(js_error)?;
        let native_context = NativeContext {
            execution: Arc::clone(&self.execution),
            class: self.host_fn_class(ctx)?,
        };
        let allowed = self.permissions.get(mounting.script_id).map(|set| &**set);
        self.registry
            .install_native_functions(
                &functions,
                mounting.script_id,
                mounting.host_promises,
                &native_context,
                allowed,
            )
            .map_err(js_error)?;
        use_context_function_prototype(ctx, &functions)?;
        Ok(functions)
    }

    /// The callable class of host functions, registered by the first script that mounts.
    fn host_fn_class(&self, ctx: &Ctx<'_>) -> Result<HostFnClass, EngineError> {
        if let Some(class) = self.host_class.get() {
            return Ok(*class);
        }
        let class = HostFnClass::register(ctx).map_err(js_error)?;
        Ok(*self.host_class.get_or_init(|| class))
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

    /// Runs the `ctx.hot.dispose` callbacks of a version no longer in `scripts`, then
    /// releases its modules and its place in its group; the context drops with the last
    /// script that holds it.
    fn retire(&mut self, script: EngineScript) -> Result<Disposed, EngineError> {
        for guard in script.request_guards.borrow_mut().drain(..) {
            guard.cancel();
        }
        self.active_tasks
            .set(self.active_tasks.get() - script.tasks.borrow().len());
        script.tasks.borrow_mut().clear();
        let disposed = self.dispose(&script);
        script.host_promises.cancel();
        self.module_store.remove_modules(&script.module_ids)?;
        self.envs.remove(script.graph_id);
        if let Some(name) = &script.group
            && let Some(group) = self.groups.get_mut(&**name)
        {
            group.scripts -= 1;
            if group.scripts == 0 {
                self.groups.remove(&**name);
            }
        }
        self.forget_unused_modules();
        Ok(disposed)
    }

    /// Calls a retired version's `ctx.hot.dispose` callbacks, in registration order.
    fn dispose(&self, script: &EngineScript) -> Disposed {
        let _budget = self.budget();
        let _charge = self.charge_to(script);
        let disposed = script
            .context
            .with(|ctx| call_hot_hook(&ctx, &script.hooks.hot_dispose).map(|_| ()));
        self.attribute_interrupt(self.settle(disposed))
    }

    /// Drops memoized modules and project states no loaded script uses anymore, so
    /// editing files during a long session does not grow them.
    fn forget_unused_modules(&mut self) {
        let modules = self
            .scripts
            .values()
            .flat_map(|script| &script.origin.module_keys)
            .map(String::as_str)
            .collect();
        let projects = self
            .scripts
            .values()
            .filter_map(|script| script.origin.project.as_ref())
            .map(|project| project.entry_path.as_path())
            .collect();
        self.transpiler.retain_used(&modules, &projects);
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

/// Calls one of the hooks of a script's `ctx.hot`.
fn call_hot_hook<'js>(
    ctx: &Ctx<'js>,
    hook: &Persistent<Function<'static>>,
) -> Result<JsValue<'js>, EngineError> {
    hook.clone()
        .restore(ctx)
        .map_err(js_error)?
        .call::<_, JsValue<'js>>(())
        .catch(ctx)
        .map_err(caught_js_error)
}

/// Reports a failed `ctx.hot` callback as [`EngineError::Execution`] whose details start
/// with `outcome`, which says what became of the script. An interrupt stays
/// [`EngineError::Interrupted`], so the host can tell the stop it asked for from a failure.
fn hot_hook_error(error: EngineError, outcome: &str) -> EngineError {
    let details = match error {
        EngineError::Interrupted => return EngineError::Interrupted,
        EngineError::Execution { details } => details,
        error => error.to_string(),
    };
    EngineError::Execution {
        details: format!("{outcome}: {details}"),
    }
}

/// A context with the core, the built-ins Manivelle needs and the optional ones `builtins`
/// asks for. All of them is `Context::full`; fewer adds the same intrinsics one by one,
/// in the order QuickJS's `JS_NewContext` does.
fn new_context(runtime: &Runtime, builtins: ScriptBuiltins) -> Result<Context, EngineError> {
    if builtins == ScriptBuiltins::ALL {
        return Context::full(runtime).map_err(js_error);
    }
    let context = Context::custom::<()>(runtime).map_err(js_error)?;
    let failed = context.with(|ctx| {
        let raw = ctx.as_raw().as_ptr();
        let failures = |enabled: bool, add: unsafe extern "C" fn(*mut qjs::JSContext) -> i32| {
            // SAFETY: `raw` is the live context `ctx` wraps. Each intrinsic is added once,
            // right after creation and before any script runs, as `JS_NewContext` does.
            enabled && unsafe { add(raw) } != 0
        };
        // `eval` compiles modules and the prelude, JSON backs `console`, `Map` backs the
        // prelude and `Promise` backs `async`: always on.
        failures(builtins.date, qjs::JS_AddIntrinsicDate)
            || failures(true, qjs::JS_AddIntrinsicEval)
            || failures(builtins.regexp, qjs::JS_AddIntrinsicRegExp)
            || failures(true, qjs::JS_AddIntrinsicJSON)
            || failures(builtins.proxy, qjs::JS_AddIntrinsicProxy)
            || failures(true, qjs::JS_AddIntrinsicMapSet)
            || failures(builtins.typed_arrays, qjs::JS_AddIntrinsicTypedArrays)
            || failures(true, qjs::JS_AddIntrinsicPromise)
            || failures(builtins.weak_ref, qjs::JS_AddIntrinsicWeakRef)
            || failures(builtins.web, qjs::JS_AddIntrinsicAToB)
            || failures(builtins.web, qjs::JS_AddPerformance)
    });
    if failed {
        Err(EngineError::Execution {
            details: String::from("out of memory while creating a script context"),
        })
    } else {
        Ok(context)
    }
}

/// The function `name` the prelude module exports, kept for the engine to call.
fn export_function<'js>(
    ctx: &Ctx<'js>,
    module: &Module<'js, Evaluated>,
    name: &str,
) -> Result<Persistent<Function<'static>>, EngineError> {
    module
        .get::<_, Function<'_>>(name)
        .map(|function| Persistent::save(ctx, function))
        .map_err(js_error)
}

/// The functions of a script's environment the engine keeps: `ctx.hot`'s `save` and
/// `dispose`, and the one that fires the script's due timers.
fn read_script_hooks<'js>(ctx: &Ctx<'js>, env: &Object<'js>) -> Result<ScriptHooks, EngineError> {
    let hook = |name: &str| {
        env.get::<_, Function<'js>>(name)
            .map(|function| Persistent::save(ctx, function))
            .map_err(js_error)
    };
    let timers_run = env.get::<_, Function<'js>>("timersRun").map_err(js_error)?;
    Ok(ScriptHooks {
        hot_save: hook("hotSave")?,
        hot_dispose: hook("hotDispose")?,
        timers_run: Retained::new(timers_run.as_value()),
    })
}

/// rquickjs gives every native function the `Function.prototype` of the first context that
/// created one, for the whole runtime: left alone, a script that patches its
/// `Function.prototype` would change the host functions of every other script, and
/// `hostFunction instanceof Function` would be false in all but that first script. Gives
/// the functions of `functions` the prototype of `ctx`.
fn use_context_function_prototype<'js>(
    ctx: &Ctx<'js>,
    functions: &Object<'js>,
) -> Result<(), EngineError> {
    let function_prototype = Function::prototype(ctx.clone());
    for entry in functions.props::<String, Function>() {
        let (_, function) = entry.map_err(js_error)?;
        function
            .set_prototype(Some(&function_prototype))
            .map_err(js_error)?;
    }
    Ok(())
}
