//! Reloading projects whose files changed, unloading scripts, and handing the `ctx.hot`
//! state from one version of a script to the next.

use std::collections::BTreeMap;

use rquickjs::{CatchResultExt, Ctx, Function, Persistent, Value as JsValue};

use crate::compiler::WatchedFiles;
use crate::contract::js_to_json;
use crate::error::EngineError;
use crate::types::{ReloadReport, ScriptId, StateSnapshot};

use super::super::errors::{caught_js_error, js_error};
use super::{Disposed, Engine, EngineScript, HotData, script_not_found};

impl Engine {
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
    pub(super) fn watch_failed_attempt(&mut self, id: &str, watched: WatchedFiles) {
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

    /// Runs the `ctx.hot.save` callback of every loaded script and collects what they
    /// return, as data a host can write to disk and later hand to [`Engine::restore`].
    ///
    /// A script that registered no `save`, or whose `save` returns `undefined`, is left
    /// out. Like at a reload, `save` must not have side effects. Nothing is made when
    /// a `save` throws or returns something JSON would lose (a function, a `Map`, a `Set`,
    /// a `Date`, a class instance, a cycle): the error names the script and the place. See
    /// [`StateSnapshot`] for what counts as plain data.
    pub fn snapshot(&self) -> Result<StateSnapshot, EngineError> {
        let mut states = BTreeMap::new();
        for (id, script) in &self.scripts {
            let saved = self.run_in_script(script, |ctx| {
                let state = call_hot_hook(&ctx, &script.hooks.hot_save)?;
                if state.is_undefined() {
                    return Ok(None);
                }
                reject_non_data(&ctx, &state)?;
                js_to_json(&ctx, state)
                    .map(Some)
                    .map_err(|error| EngineError::Execution {
                        details: error.to_string(),
                    })
            });
            let saved = saved.map_err(|error| {
                hot_hook_error(
                    error,
                    &format!("`ctx.hot.save` of script `{id}` failed; no snapshot was made"),
                )
            })?;
            if let Some(state) = saved {
                states.insert(id.to_string(), state);
            }
        }
        Ok(StateSnapshot(states))
    }

    /// Keeps the states of `snapshot` for the scripts that are not loaded yet: each one's
    /// first load starts with its state as `ctx.hot.data`, as if a previous version had
    /// saved it. A state is used once, by the first load that succeeds; a failed load
    /// leaves it for the next try. A state for a script that never loads stays unused.
    ///
    /// Call it before loading the scripts. A snapshot that holds a script already loaded
    /// fails with [`EngineError::ScriptAlreadyLoaded`], and none of its states is kept.
    pub fn restore(&mut self, snapshot: StateSnapshot) -> Result<(), EngineError> {
        if let Some(id) = snapshot
            .0
            .keys()
            .find(|id| self.scripts.contains_key(id.as_str()))
        {
            return Err(EngineError::ScriptAlreadyLoaded {
                script_id: ScriptId::from(id.as_str()),
            });
        }
        self.restored.extend(
            snapshot
                .0
                .into_iter()
                .map(|(id, state)| (ScriptId::from(id), state)),
        );
        Ok(())
    }

    /// Runs the loaded version's `ctx.hot.save` callback, when the script is loaded;
    /// its result becomes the next version's `ctx.hot.data`.
    pub(super) fn save_hot_data(&self, id: &str) -> Result<Option<HotData>, EngineError> {
        let Some(script) = self.scripts.get(id) else {
            return Ok(None);
        };
        let saved = self.run_in_script(script, |ctx| {
            call_hot_hook(&ctx, &script.hooks.hot_save).map(|data| Persistent::save(&ctx, data))
        });
        saved.map(Some).map_err(|error| {
            hot_hook_error(
                error,
                &format!(
                    "`ctx.hot.save` of script `{id}` failed; the previous version keeps running"
                ),
            )
        })
    }

    /// Runs the `ctx.hot.dispose` callbacks of a version no longer in `scripts`, then
    /// releases its modules and its place in its group; the context drops with the last
    /// script that holds it.
    pub(super) fn retire(&mut self, script: EngineScript) -> Result<Disposed, EngineError> {
        for guard in script.request_guards.borrow_mut().drain(..) {
            guard.cancel();
        }
        self.scheduling
            .active_tasks
            .set(self.scheduling.active_tasks.get() - script.tasks.borrow().len());
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
        self.run_in_script(script, |ctx| {
            call_hot_hook(&ctx, &script.hooks.hot_dispose).map(|_| ())
        })
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

/// Looks for the first value of a saved state that JSON cannot hold faithfully: a function,
/// a symbol, a BigInt, a `Map`, a `Set`, a `Date`, a class instance, or an object that
/// contains itself. The JSON conversion would turn a function, a `Map`, a `Set` or a `Date`
/// into `{}` without a word, and a snapshot that loses data silently is worse than none.
/// Evaluates to the place of the first one, or `undefined` for plain data.
const FIND_NON_DATA: &str = r#"(value) => {
  const ancestors = [];
  const walk = (v, path) => {
    if (v === null || v === undefined) return undefined;
    const kind = typeof v;
    if (kind === "boolean" || kind === "string" || kind === "number") return undefined;
    if (kind !== "object") return `${path}: a ${kind}`;
    if (ancestors.includes(v)) return `${path}: a reference to itself`;
    const array = Array.isArray(v);
    const proto = Object.getPrototypeOf(v);
    if (!array && proto !== Object.prototype && proto !== null) {
      return `${path}: ${v.constructor?.name ?? "an object"} is not plain data`;
    }
    ancestors.push(v);
    let found;
    if (array) {
      for (let i = 0; i < v.length && found === undefined; i++) {
        found = walk(v[i], `${path}[${i}]`);
      }
    } else {
      for (const key of Object.keys(v)) {
        found = walk(v[key], `${path}.${key}`);
        if (found !== undefined) break;
      }
    }
    ancestors.pop();
    return found;
  };
  return walk(value, "state");
}"#;

/// Fails, naming where, when `state` holds something JSON cannot carry faithfully.
fn reject_non_data<'js>(ctx: &Ctx<'js>, state: &JsValue<'js>) -> Result<(), EngineError> {
    let find = ctx
        .eval::<Function<'js>, _>(FIND_NON_DATA)
        .catch(ctx)
        .map_err(caught_js_error)?;
    let place = find
        .call::<_, Option<String>>((state.clone(),))
        .catch(ctx)
        .map_err(caught_js_error)?;
    match place {
        Some(place) => Err(EngineError::Execution {
            details: format!("the saved state is not plain data at {place}"),
        }),
        None => Ok(()),
    }
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
pub(super) fn hot_hook_error(error: EngineError, outcome: &str) -> EngineError {
    let details = match error {
        EngineError::Interrupted => return EngineError::Interrupted,
        EngineError::Execution { details } => details,
        error => error.to_string(),
    };
    EngineError::Execution {
        details: format!("{outcome}: {details}"),
    }
}
