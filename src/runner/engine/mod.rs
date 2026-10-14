//! Single-thread engine: the owning thread runs QuickJS, and every call crosses the
//! Rust/JS boundary natively, without generated source or JSON text.

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;
use std::time::Duration;

use indexmap::IndexMap;
use rquickjs::{Context, Ctx, Function, Persistent, Runtime, Value, Value as JsValue, qjs};
use rustc_hash::FxBuildHasher;

use crate::compiler::WatchedFiles;
use crate::config::{EngineOptions, ScriptBuiltins};
use crate::error::EngineError;
use crate::registry::InMemoryHostContractRegistry;
use crate::types::{MemoryStats, ScriptId};

use super::console::{ConsoleLevel, ConsoleSink};
use super::errors::{js_error, pending_exception};
use super::events::{EventIndex, ListenedEvents};
use super::execution::{ExecutionControl, ExecutionGuard};
use super::host_fn::HostFnClass;
use super::host_promises::HostPromises;
use super::interrupt::InterruptHandle;
use super::memory::memory_stats;
use super::module_loader::{MemoryModuleLoader, MemoryModuleResolver, ModuleStore, ScriptEnvs};
use super::promise_rejections::UnhandledRejections;
use super::quota::{Owner, Quota, QuotaAllocator};
use super::retained::{KeptAtoms, Retained};
use super::tasks::{RequestGuard, ScriptTask};
use super::timers::{EarliestDue, NextTimer, TimerClock};
use super::transpile::Transpiler;
use super::web::WebBytecode;

mod delivery;
mod dispatch;
mod faults;
mod host_driven;
mod invoke;
mod lifecycle;

/// Single-thread Manivelle engine.
///
/// The engine owns its QuickJS runtime and cannot leave the thread that created it.
/// Script exports are called directly, host functions are native QuickJS functions,
/// and values convert without JSON text.
///
/// Every load, call, emit, request and timer advance runs under
/// `EngineOptions::execution_timeout`: JavaScript still running when it expires is
/// interrupted and the operation fails with `EngineError::Execution`.
/// [`Engine::interrupt_handle`] stops it earlier from another thread. Both are
/// cooperative and cannot preempt a Rust host function.
///
/// Promise jobs an operation queues run before it returns, within the same budget.
/// A synchronous [`Engine::call`] or [`Engine::request`] waits for an `async` export
/// only while script jobs can settle it. For host or timer-driven work, use
/// [`Engine::call_deferred`] or [`Engine::request_deferred`] and drive continuations
/// with [`Engine::pump`] or [`Engine::advance_timers`]. Unhandled rejections fail
/// the operation; observed deferred rejections belong to their pending handle.
///
/// Scripts get `setTimeout`, `setInterval`, `clearTimeout` and `clearInterval` on an
/// engine clock that only [`Engine::advance_timers`] moves, and a `console` whose
/// output goes to stderr unless [`Engine::set_console`] routes it elsewhere.
///
/// Replacing a loaded script, through [`Engine::load_script`], [`Engine::load_project`]
/// or [`Engine::reload_changed`], hands over the state the script opts to keep with
/// `ctx.hot`: the loaded version's `ctx.hot.save` callback runs first, and its result
/// is the new version's `ctx.hot.data` before any of its code runs. Once the new
/// version is loaded, the old version's `ctx.hot.dispose` callbacks run, in
/// registration order. A throwing `save` fails the load and keeps the old version; a
/// throwing `dispose` fails the load call although the new version stays loaded.
/// [`Engine::unload_script`] runs the `dispose` callbacks too; dropping the engine
/// does not.
pub struct Engine {
    /// In load order, which is the order events are delivered in.
    scripts: IndexMap<ScriptId, EngineScript, FxBuildHasher>,
    active_tasks: Cell<usize>,
    registry: Arc<InMemoryHostContractRegistry>,
    transpiler: Transpiler,
    module_store: ModuleStore,
    next_graph_id: u64,
    execution: Arc<ExecutionControl>,
    execution_timeout: Duration,
    max_script_faults: u32,
    /// Whether a script already faulted for lack of memory in the running operation: the
    /// pool is shared, so the scripts that fail after the first are not counted.
    memory_fault_seen: Cell<bool>,
    /// The books of the per-script memory quota; `None` when no quota is set.
    quota: Option<Rc<Quota>>,
    console: ConsoleSink,
    timer_clock: TimerClock,
    /// Which scripts listen to each event, for `emit` and `request`.
    event_index: EventIndex,
    /// No later than the earliest due time of any script's timers.
    earliest_timer: EarliestDue,
    /// Raised when a resolver queues an answer for any script, lowered by `pump`.
    host_wake: Arc<AtomicBool>,
    builtins: ScriptBuiltins,
    /// Bytecode of the context prelude, compiled by the first script that mounts.
    prelude: OnceCell<Box<[u8]>>,
    /// The callable class of host functions, registered by the first script that mounts.
    host_class: OnceCell<HostFnClass>,
    /// The atoms of the export names the host calls.
    export_atoms: KeptAtoms,
    /// Bytecode of the web APIs module, compiled by the first context that uses them.
    web_bytecode: WebBytecode,
    /// Contexts shared by scripts, by group name.
    groups: HashMap<Box<str>, ScriptGroup, FxBuildHasher>,
    /// The environment of every script loaded into a group, for its `manivelle:env` module.
    envs: ScriptEnvs,
    // Declared last: contexts and persistent values must drop before their runtime.
    rejections: UnhandledRejections,
    runtime: Runtime,
}

struct EngineScript {
    /// The script's own context, or the one its group shares.
    context: Context,
    /// The group whose context it shares; `None` for a context of its own.
    group: Option<Box<str>>,
    graph_id: u64,
    hooks: ScriptHooks,
    /// The module namespace object, read for the export a call names.
    exports: Retained,
    host_promises: HostPromises,
    tasks: RefCell<Vec<ScriptTask>>,
    request_guards: RefCell<Vec<RequestGuard>>,
    signals: ScriptSignals,
    module_ids: Vec<String>,
    origin: ScriptOrigin,
    /// How many times the script ran out of its time or memory while an event, a timer or
    /// an answer was delivered to it.
    faults: Cell<u32>,
    /// The script's seat in the memory quota's books.
    owner: Option<Owner>,
    /// Whether it faulted `max_script_faults` times: its handlers and timers no longer
    /// run. A reload makes a new script, which starts with none.
    quarantined: Cell<bool>,
}

impl EngineScript {
    /// Whether both scripts run in the same context: they are in the same group.
    fn shares_context(&self, other: &Self) -> bool {
        self.group.is_some() && self.group == other.group
    }
}

/// The functions of a script's environment the engine calls: `ctx.hot`'s `save` and
/// `dispose`, and the one that fires the script's due timers.
struct ScriptHooks {
    hot_save: Persistent<Function<'static>>,
    hot_dispose: Persistent<Function<'static>>,
    timers_run: Retained,
}

/// The functions the context prelude exports, which build and expose a script's
/// environment.
#[derive(Clone)]
struct Prelude {
    make_env: Persistent<Function<'static>>,
    install_globals: Persistent<Function<'static>>,
}

/// A context shared by the scripts loaded into one group, and how many they are; the
/// context goes with the last of them.
struct ScriptGroup {
    context: Context,
    prelude: Prelude,
    scripts: usize,
}

/// What a script's context reports to the engine as it registers handlers and timers,
/// so `emit` and `advance_timers` skip scripts with nothing to run instead of entering
/// their context, and find the handlers to run without a lookup by name. Owned by the
/// script: a reload starts anew.
struct ScriptSignals {
    events: ListenedEvents,
    next_timer: NextTimer,
}

impl ScriptSignals {
    fn new(events: &EventIndex) -> Self {
        Self {
            events: ListenedEvents::new(events.stale_flag()),
            next_timer: NextTimer::default(),
        }
    }
}

/// The value a replaced version's `ctx.hot.save` returned, on its way to the next
/// version's `ctx.hot.data`. Contexts share the runtime, so it is the object itself.
type HotData = Persistent<JsValue<'static>>;

/// Outcome of retiring a replaced or unloaded version: its `ctx.hot.dispose` error.
type Disposed = Result<(), EngineError>;

/// Where a script's code came from, kept for hot reload and the transpile memo.
struct ScriptOrigin {
    /// Content keys of its transpiled modules.
    module_keys: Vec<String>,
    /// Entry file and watched files of a project; `None` for inline scripts.
    project: Option<ProjectFiles>,
}

struct ProjectFiles {
    entry_path: PathBuf,
    watched: WatchedFiles,
}

impl Engine {
    /// Creates an engine on the current thread using the memory, stack, cache and
    /// validation settings from `options`.
    pub fn new(options: &EngineOptions) -> Result<Self, EngineError> {
        options.validate()?;
        let module_store = ModuleStore::default();
        let execution = Arc::new(ExecutionControl::default());
        let envs = ScriptEnvs::default();
        let quota = (options.script_memory_limit_bytes != usize::MAX)
            .then(|| Quota::new(options.script_memory_limit_bytes));
        let runtime = new_runtime(options, quota.as_ref(), &module_store, &execution, &envs)?;
        Ok(Self {
            scripts: IndexMap::default(),
            active_tasks: Cell::new(0),
            registry: Arc::new(InMemoryHostContractRegistry::with_validation_options(
                options.contract_validation,
                options.unknown_field_validation,
            )),
            transpiler: Transpiler::new(options.cache_dir.as_deref())?,
            module_store,
            next_graph_id: 0,
            execution,
            execution_timeout: options.execution_timeout,
            max_script_faults: options.max_script_faults,
            memory_fault_seen: Cell::new(false),
            quota,
            console: ConsoleSink::default(),
            timer_clock: TimerClock::default(),
            earliest_timer: EarliestDue::default(),
            event_index: EventIndex::default(),
            host_wake: Arc::default(),
            builtins: options.builtins,
            prelude: OnceCell::new(),
            host_class: OnceCell::new(),
            export_atoms: KeptAtoms::default(),
            web_bytecode: WebBytecode::default(),
            groups: HashMap::default(),
            envs,
            rejections: UnhandledRejections::install(&runtime),
            runtime,
        })
    }

    /// Host contract registry. Register contracts before loading the scripts that use them.
    pub fn registry(&self) -> &InMemoryHostContractRegistry {
        &self.registry
    }

    /// Routes the `console` output of every script, loaded or to come, to `sink`, which
    /// receives the level, the id of the calling script and the message: the
    /// arguments of the call joined by spaces, strings as they are, other values as
    /// JSON when they have one. Stack locations point at the TypeScript source.
    /// Without a sink, output goes to stderr as `[level] script: message`.
    pub fn set_console(&mut self, sink: impl Fn(ConsoleLevel, &str, &str) + 'static) {
        self.console.replace(sink);
    }

    /// QuickJS memory counters for the whole engine.
    pub fn memory_stats(&self) -> MemoryStats {
        memory_stats(self.runtime.memory_usage())
    }

    /// The QuickJS memory, in bytes, the script holds against its quota
    /// (`EngineOptions::script_memory_limit_bytes`): what it allocated while it ran and
    /// has not freed, its context included. Cycles it left are counted until
    /// [`Engine::run_gc`] collects them. `None` when the engine has no quota; an error
    /// when the script is not loaded.
    pub fn script_memory_bytes(&self, script_id: &str) -> Result<Option<u64>, EngineError> {
        let script = self.script(script_id)?;
        Ok(script.owner.as_ref().map(|owner| owner.used() as u64))
    }

    /// Collects now the garbage that reference counting cannot free: objects that only
    /// reference each other in a cycle. Everything else is freed as soon as it is no
    /// longer referenced. Call it where a pause does not matter, such as a loading or
    /// results screen, typically with automatic collection off
    /// ([`Engine::set_gc_threshold`]).
    pub fn run_gc(&self) {
        self.runtime.run_gc();
    }

    /// Sets the allocated memory, in bytes, past which QuickJS collects cycles on its own
    /// when a script next creates an object; `None` turns automatic collection off. The
    /// default is 256 KiB, and after each automatic collection QuickJS moves the
    /// threshold to 1.5 times the memory still in use.
    ///
    /// A collection pauses the script that triggered it for a time that grows with
    /// the number of live objects in the whole engine. To keep it out of time-critical
    /// stretches, such as a song, turn it off for their duration and call
    /// [`Engine::run_gc`] afterwards. Cyclic garbage then accumulates meanwhile: watch
    /// [`Engine::memory_stats`], since reaching `EngineOptions::memory_limit_bytes` fails
    /// the operation that allocates.
    pub fn set_gc_threshold(&self, threshold: Option<usize>) {
        self.runtime
            .set_gc_threshold(threshold.unwrap_or(usize::MAX));
    }

    /// A handle that stops this engine's running JavaScript from any thread.
    pub fn interrupt_handle(&self) -> InterruptHandle {
        InterruptHandle::new(Arc::clone(&self.execution))
    }

    fn script(&self, id: &str) -> Result<&EngineScript, EngineError> {
        self.scripts.get(id).ok_or_else(|| script_not_found(id))
    }

    /// Starts the execution budget for one operation; it ends when the guard drops.
    fn budget(&self) -> ExecutionGuard<'_> {
        self.memory_fault_seen.set(false);
        self.execution.enter(self.execution_timeout)
    }
}

impl EngineScript {
    /// The function the script exports as `name`, read now: a binding the module
    /// reassigns is what the next call finds. The module's namespace is read through the C
    /// API with the name's kept atom, which takes no reference to the namespace or its
    /// context.
    fn export<'js>(
        &self,
        atoms: &KeptAtoms,
        ctx: &Ctx<'js>,
        script_id: &str,
        name: &str,
    ) -> Result<Function<'js>, EngineError> {
        let raw = ctx.as_raw().as_ptr();
        // SAFETY: `raw` is the live context of `ctx` and the namespace a value of it; the
        // atom is valid for the call, and the value `JS_GetProperty` returns is owned.
        let value = atoms
            .with(ctx, name, |atom| unsafe {
                qjs::JS_GetProperty(raw, self.exports.raw(), atom)
            })
            .ok_or(rquickjs::Error::Allocation)
            .map_err(js_error)?;
        // SAFETY: `value` is an owned value of `ctx`, which the wrapper releases.
        let value = unsafe {
            if qjs::JS_IsException(value) {
                return Err(pending_exception(ctx));
            }
            Value::from_raw(ctx.clone(), value)
        };
        value
            .into_function()
            .ok_or_else(|| EngineError::FunctionNotFound {
                script_id: script_id.to_owned(),
                function_name: name.to_owned(),
            })
    }
}

fn new_runtime(
    options: &EngineOptions,
    quota: Option<&Rc<Quota>>,
    module_store: &ModuleStore,
    execution: &Arc<ExecutionControl>,
    envs: &ScriptEnvs,
) -> Result<Runtime, EngineError> {
    let runtime = match quota {
        Some(quota) => Runtime::new_with_alloc(QuotaAllocator::new(Rc::clone(quota))),
        None => Runtime::new(),
    }
    .map_err(js_error)?;
    runtime.set_memory_limit(options.memory_limit_bytes);
    runtime.set_max_stack_size(options.max_stack_size_bytes);
    runtime.set_loader(
        MemoryModuleResolver::new(module_store.clone()),
        MemoryModuleLoader::new(module_store.clone(), envs.handle()),
    );
    let interrupt = Arc::clone(execution);
    runtime.set_interrupt_handler(Some(Box::new(move || interrupt.interrupted())));
    Ok(runtime)
}

fn script_not_found(id: &str) -> EngineError {
    EngineError::ScriptNotFound {
        script_id: id.to_owned(),
    }
}
