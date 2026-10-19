//! The environment a script runs in: its host functions, its hooks, and the context it is
//! mounted into.

use std::sync::Arc;

use rquickjs::{
    CatchResultExt, Context, Ctx, Function, Object, Persistent, Runtime, Value as JsValue, qjs,
};

use crate::config::ScriptBuiltins;
use crate::error::EngineError;
use crate::registry::NativeContext;

use super::super::errors::{caught_js_error, js_error};
use super::super::host_fn::HostFnClass;
use super::super::retained::Retained;
use super::super::timers::timer_hooks;
use super::super::web;
use super::mount::Mounting;
use super::{Engine, HotData, Prelude, ScriptHooks};

impl Engine {
    /// Builds the environment of a script: its host functions, bound to its id, the hooks
    /// through which its handlers and timers report, its `console` sink and the state the
    /// previous version saved. Returns it with the functions the engine calls.
    pub(super) fn build_env<'js>(
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
            &self.scheduling.timer_clock,
            &signals.next_timer,
            &self.scheduling.earliest_timer,
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
        if let Some(class) = self.shared.host_class.get() {
            return Ok(*class);
        }
        let class = HostFnClass::register(ctx).map_err(js_error)?;
        Ok(*self.shared.host_class.get_or_init(|| class))
    }
}

/// A context with the core, the built-ins Manivelle needs and the optional ones `builtins`
/// asks for. All of them is `Context::full`; fewer adds the same intrinsics one by one,
/// in the order QuickJS's `JS_NewContext` does.
pub(super) fn new_context(
    runtime: &Runtime,
    builtins: ScriptBuiltins,
) -> Result<Context, EngineError> {
    if builtins == ScriptBuiltins::ALL {
        return Context::full(runtime).map_err(js_error);
    }
    let context = Context::custom::<()>(runtime).map_err(js_error)?;
    // `eval` compiles modules and the prelude, JSON backs `console`, `Map` backs the
    // prelude and `Promise` backs `async`: always on.
    let intrinsics: [(bool, unsafe extern "C" fn(*mut qjs::JSContext) -> i32); 11] = [
        (builtins.date, qjs::JS_AddIntrinsicDate),
        (true, qjs::JS_AddIntrinsicEval),
        (builtins.regexp, qjs::JS_AddIntrinsicRegExp),
        (true, qjs::JS_AddIntrinsicJSON),
        (builtins.proxy, qjs::JS_AddIntrinsicProxy),
        (true, qjs::JS_AddIntrinsicMapSet),
        (builtins.typed_arrays, qjs::JS_AddIntrinsicTypedArrays),
        (true, qjs::JS_AddIntrinsicPromise),
        (builtins.weak_ref, qjs::JS_AddIntrinsicWeakRef),
        (builtins.web, qjs::JS_AddIntrinsicAToB),
        (builtins.web, qjs::JS_AddPerformance),
    ];
    let failed = context.with(|ctx| {
        let raw = ctx.as_raw().as_ptr();
        intrinsics.iter().any(|&(enabled, add)| {
            // SAFETY: `raw` is the live context `ctx` wraps. Each intrinsic is added once,
            // right after creation and before any script runs, as `JS_NewContext` does.
            enabled && unsafe { add(raw) } != 0
        })
    });
    if failed {
        Err(EngineError::Execution {
            details: String::from("out of memory while creating a script context"),
        })
    } else {
        Ok(context)
    }
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
    for entry in functions.props::<String, Function>() {
        let (_, function) = entry.map_err(js_error)?;
        web::use_context_prototype(ctx, &function)?;
    }
    Ok(())
}
