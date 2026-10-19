//! The `console` object of scripts, routed to a host sink.

use std::cell::{Cell, RefCell};
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
pub(super) struct ConsoleSink(Rc<Sinks>);

/// The sink the host chose, and a tap that sees every line as well; `set_console`
/// replaces the sink and leaves the tap.
struct Sinks {
    sink: RefCell<Box<SinkFn>>,
    tap: RefCell<Option<Box<SinkFn>>>,
    /// Whether `sink` is still the stderr writer nobody replaced.
    default: Cell<bool>,
}

fn to_stderr(level: ConsoleLevel, script_id: &str, message: &str) {
    eprintln!("[{level}] {script_id}: {message}");
}

impl Default for ConsoleSink {
    /// Writes `[level] script: message` lines to stderr.
    fn default() -> Self {
        Self(Rc::new(Sinks {
            sink: RefCell::new(Box::new(to_stderr)),
            tap: RefCell::new(None),
            default: Cell::new(true),
        }))
    }
}

impl ConsoleSink {
    pub(super) fn replace(&self, sink: impl Fn(ConsoleLevel, &str, &str) + 'static) {
        *self.0.sink.borrow_mut() = Box::new(sink);
        self.0.default.set(false);
    }

    /// Passes every line to `forward` as well, whatever sink the host sets. While the
    /// host has not chosen a sink, the default stderr writer is silenced: the receiver
    /// of `forward` shows the lines instead of the program.
    pub(super) fn forward(&self, forward: impl Fn(ConsoleLevel, &str, &str) + 'static) {
        *self.0.tap.borrow_mut() = Some(Box::new(forward));
        if self.0.default.get() {
            *self.0.sink.borrow_mut() = Box::new(|_, _, _| {});
        }
    }

    /// Ends [`forward`](Self::forward), and gives the stderr writer back if it was silenced.
    pub(super) fn stop_forwarding(&self) {
        *self.0.tap.borrow_mut() = None;
        if self.0.default.get() {
            *self.0.sink.borrow_mut() = Box::new(to_stderr);
        }
    }

    /// The native function the prelude's `console` of script `script_id` writes to;
    /// stack locations in messages point at the TypeScript source.
    pub(super) fn writer<'js>(
        &self,
        ctx: &Ctx<'js>,
        script_id: &str,
        modules: &ModuleStore,
    ) -> Result<Function<'js>, EngineError> {
        let sinks = Rc::clone(&self.0);
        let script_id = Box::<str>::from(script_id);
        let modules = modules.clone();
        Function::new(ctx.clone(), move |method: String, message: String| {
            let message = locations_in_typescript(&message, &modules);
            host_fn::guarded(|| {
                let level = ConsoleLevel::from_method(&method);
                (sinks.sink.borrow())(level, &script_id, &message);
                if let Some(tap) = sinks.tap.borrow().as_ref() {
                    tap(level, &script_id, &message);
                }
            })
        })
        .map_err(js_error)
    }
}
