//! The `console` object of scripts, routed to a host sink.

use std::cell::RefCell;
use std::fmt;
use std::rc::Rc;

use rquickjs::{Ctx, Function};

use super::errors::{js_error, locations_in_typescript};
use super::host_fn;
use super::module_loader::ModuleStore;
use crate::error::EngineError;

/// Severity of one `console` call, from the method a script called.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ConsoleLevel {
    /// `console.debug`.
    Debug,
    /// `console.log`.
    Log,
    /// `console.info`.
    Info,
    /// `console.warn`.
    Warn,
    /// `console.error`.
    Error,
}

impl ConsoleLevel {
    /// The level of the `console` method named `method`; the prelude only passes these.
    fn from_method(method: &str) -> Self {
        match method {
            "debug" => Self::Debug,
            "info" => Self::Info,
            "warn" => Self::Warn,
            "error" => Self::Error,
            _ => Self::Log,
        }
    }
}

impl fmt::Display for ConsoleLevel {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Debug => "debug",
            Self::Log => "log",
            Self::Info => "info",
            Self::Warn => "warn",
            Self::Error => "error",
        })
    }
}

type SinkFn = dyn Fn(ConsoleLevel, &str, &str);

/// Where every script's `console` output goes; shared by all contexts, so replacing
/// it also reroutes the scripts already loaded.
#[derive(Clone)]
pub(super) struct ConsoleSink(Rc<RefCell<Box<SinkFn>>>);

impl Default for ConsoleSink {
    /// Writes `[level] script: message` lines to stderr.
    fn default() -> Self {
        Self::new(|level, script_id, message| eprintln!("[{level}] {script_id}: {message}"))
    }
}

impl ConsoleSink {
    fn new(sink: impl Fn(ConsoleLevel, &str, &str) + 'static) -> Self {
        Self(Rc::new(RefCell::new(Box::new(sink))))
    }

    pub(super) fn replace(&self, sink: impl Fn(ConsoleLevel, &str, &str) + 'static) {
        *self.0.borrow_mut() = Box::new(sink);
    }

    /// The native function the prelude's `console` of script `script_id` writes to;
    /// stack locations in messages point at the TypeScript source.
    pub(super) fn writer<'js>(
        &self,
        ctx: &Ctx<'js>,
        script_id: &str,
        modules: &ModuleStore,
    ) -> Result<Function<'js>, EngineError> {
        let sink = Rc::clone(&self.0);
        let script_id = Box::<str>::from(script_id);
        let modules = modules.clone();
        Function::new(ctx.clone(), move |method: String, message: String| {
            let message = locations_in_typescript(&message, &modules);
            host_fn::guarded(|| {
                (sink.borrow())(ConsoleLevel::from_method(&method), &script_id, &message);
            })
        })
        .map_err(js_error)
    }
}
