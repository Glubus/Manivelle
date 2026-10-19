//! The context prelude every script context evaluates, the web APIs it declares, and the
//! functions it exports for the engine to call.

use rquickjs::module::{Declared, Evaluated};
use rquickjs::{CatchResultExt, Ctx, Function, Module, Persistent, WriteOptions};

use crate::error::EngineError;

use super::super::errors::{caught_js_error, js_error};
use super::super::web;
use super::{Engine, Prelude};

/// Builds `ctx.on` / `ctx.off` and their handler lists, `ctx.hot`, `console` and the timer
/// functions of one script, and can expose them, with `__host` and the namespaced host
/// globals, as globals. A module, so that its compiled form can be kept and loaded into
/// every context.
const CONTEXT_PRELUDE: &str = include_str!("../../../assets/context_prelude.js");
const PRELUDE_MODULE: &str = "manivelle:prelude";

/// What `import ... from "manivelle:env"` reads in a context of its own: the globals the
/// prelude exposed. A script in a group gets its own environment module instead.
pub(super) const GLOBALS_ENV_MODULE: &str = r#"
export const ctx = globalThis.ctx;
export const console = globalThis.console;
export const setTimeout = globalThis.setTimeout;
export const setInterval = globalThis.setInterval;
export const clearTimeout = globalThis.clearTimeout;
export const clearInterval = globalThis.clearInterval;
"#;

impl Engine {
    /// Runs the context prelude in `ctx` and declares the web APIs it can load.
    pub(super) fn run_prelude(&self, ctx: &Ctx<'_>) -> Result<Prelude, EngineError> {
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
        if let Some(bytecode) = self.shared.prelude.get() {
            // SAFETY: QuickJS does not verify bytecode. These bytes were written by this
            // process, from the crate's own prelude, by the same QuickJS build.
            return unsafe { Module::load(ctx.clone(), bytecode) };
        }
        let declared = Module::declare(ctx.clone(), PRELUDE_MODULE, CONTEXT_PRELUDE)?;
        if let Ok(bytecode) = declared.write(WriteOptions::default()) {
            // Only this thread sets it, once: the first mount.
            let _ = self.shared.prelude.set(bytecode.into_boxed_slice());
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
                web::loader(ctx, &self.shared.web_bytecode)?,
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
