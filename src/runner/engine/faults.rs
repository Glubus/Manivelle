//! Scripts that run out of their budget while the host delivers to them: each script
//! of an emission, a timer pass or a pump has a budget of its own (a *slice*), and one that
//! keeps running out of it is quarantined.

use std::mem;
use std::ptr::{self, NonNull};
use std::time::Duration;

use rquickjs::{Ctx, qjs};

use super::super::errors::{OUT_OF_MEMORY, pending_exception};
use super::dispatch::NULL_EXCEPTION;
use super::{Engine, EngineScript};
use crate::error::EngineError;

/// Why a script's slice counts as a fault.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Fault {
    /// It used up its time.
    Budget,
    /// It failed for lack of memory.
    Memory,
}

/// Where a script stands after a fault was counted against it.
#[derive(Clone, Copy, PartialEq, Eq)]
enum Standing {
    /// Quarantine is off (`max_script_faults` is 0).
    Unlimited,
    /// It has `faults` of the `allowed` ones.
    Warned { faults: u32, allowed: u32 },
    /// It reached the limit: its handlers and timers no longer run.
    Quarantined { faults: u32 },
}

impl Engine {
    /// The ids, in load order, of the scripts that ran out of their time or memory
    /// budget [`EngineOptions::max_script_faults`](crate::EngineOptions::max_script_faults)
    /// times while an event, a timer or an answer was delivered to them. Their handlers
    /// and timers no longer run, and their pending host answers were dropped; `call`
    /// still reaches their exports. Reloading a script with `load_script` (or another
    /// `load_*`) replaces it with a fresh one, and unloading it also frees what it holds.
    pub fn quarantined_scripts(&self) -> impl Iterator<Item = &str> {
        self.scripts
            .iter()
            .filter(|(_, script)| script.quarantined.get())
            .map(|(id, _)| id.as_str())
    }

    /// Runs `body`, which delivers something to `script` in `ctx`, as one slice: under a
    /// budget of its own, followed by the Promise jobs it queued, and judges the outcome.
    /// `what` says what was delivered, for the error that names the script.
    ///
    /// Runs inside the script's context, so it must not take the runtime lock again: the
    /// memory pool is measured through `ctx` rather than through `memory_stats`.
    pub(super) fn run_slice(
        &self,
        ctx: &Ctx<'_>,
        id: &str,
        script: &EngineScript,
        what: impl FnOnce() -> String,
        body: impl FnOnce() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        self.execution.restart_slice();
        let outcome = body();
        let jobs = drain_jobs(ctx);
        self.judge_slice(ctx, id, script, outcome.and(jobs), what)
    }

    /// Starts a budget for the work that follows a multi-script operation (its settling).
    #[inline]
    pub(super) fn begin_settle_slice(&self) {
        self.execution.restart_slice();
    }

    /// Ends the slice of `script`, which delivered with `outcome`: when it ran out of its
    /// time, or failed for lack of memory, that is a fault, and the error names the
    /// script. Any other outcome is returned as it is. The caller records the error and
    /// goes on to the next script unless the host asked to stop.
    fn judge_slice(
        &self,
        ctx: &Ctx<'_>,
        id: &str,
        script: &EngineScript,
        outcome: Result<(), EngineError>,
        what: impl FnOnce() -> String,
    ) -> Result<(), EngineError> {
        let Some(fault) = self.fault_of(ctx, &outcome) else {
            return outcome;
        };
        let standing = self.count_fault(script);
        Err(EngineError::Execution {
            details: fault_message(id, &what(), fault, standing, self.execution_timeout),
        })
    }

    /// The fault, if any, that the slice that just ended with `outcome` committed.
    ///
    /// The memory pool is shared, so a script that fails to allocate may be the victim of
    /// another one that holds the memory. Only the first script to fail for lack of memory
    /// in an operation is counted: those after it are reported, but not faulted.
    fn fault_of(&self, ctx: &Ctx<'_>, outcome: &Result<(), EngineError>) -> Option<Fault> {
        if self.execution.slice_expired() {
            return Some(Fault::Budget);
        }
        let Err(EngineError::Execution { details }) = outcome else {
            return None;
        };
        let ran_out_of_memory = details.starts_with(OUT_OF_MEMORY)
            || (looks_like_exhaustion(details) && pool_nearly_full(ctx));
        (ran_out_of_memory && !self.memory_fault_seen.replace(true)).then_some(Fault::Memory)
    }

    /// Counts a fault against `script`, quarantining it at the limit.
    fn count_fault(&self, script: &EngineScript) -> Standing {
        let faults = script.faults.get() + 1;
        script.faults.set(faults);
        let allowed = self.max_script_faults;
        if allowed == 0 {
            return Standing::Unlimited;
        }
        if faults < allowed {
            return Standing::Warned { faults, allowed };
        }
        self.quarantine(script);
        Standing::Quarantined { faults }
    }

    /// Takes `script` out of the event index, its timers and the pump.
    fn quarantine(&self, script: &EngineScript) {
        if !script.quarantined.replace(true) {
            self.event_index.mark_stale();
            script.host_promises.cancel();
        }
    }

    /// Whether an error message says the engine ran out of memory, as the engine words it
    /// when it is near its limit. Takes the runtime lock: not for use inside a context.
    pub(super) fn is_out_of_memory(&self, details: &str) -> bool {
        if !looks_like_exhaustion(details) {
            return false;
        }
        let stats = self.memory_stats();
        stats.malloc_limit_bytes != 0
            && stats.malloc_size_bytes * 10 >= stats.malloc_limit_bytes * 9
    }
}

/// Whether an error message is one QuickJS gives when an allocation fails while it was
/// already handling a failure: nothing, or a thrown `null`.
fn looks_like_exhaustion(details: &str) -> bool {
    details.is_empty() || details.starts_with(NULL_EXCEPTION) || details.starts_with(OUT_OF_MEMORY)
}

/// Whether the engine's memory pool, measured from `ctx`, is at least nine tenths full.
/// Reads the runtime directly: this runs inside a context, where `memory_stats` would take
/// the runtime lock a second time.
fn pool_nearly_full(ctx: &Ctx<'_>) -> bool {
    // SAFETY: `JSMemoryUsage` is a plain struct of integers, for which all zeroes is valid.
    let mut usage: qjs::JSMemoryUsage = unsafe { mem::zeroed() };
    // SAFETY: `ctx` is a live context, so its runtime is alive, and `usage` is writable.
    unsafe { qjs::JS_ComputeMemoryUsage(qjs::JS_GetRuntime(ctx.as_raw().as_ptr()), &mut usage) };
    usage.malloc_limit != 0 && usage.malloc_size * 10 >= usage.malloc_limit * 9
}

/// The error for a script that faulted: what it was doing, why it failed, and where it
/// stands.
fn fault_message(
    id: &str,
    what: &str,
    fault: Fault,
    standing: Standing,
    timeout: Duration,
) -> String {
    let cause = match fault {
        Fault::Budget => format!("ran out of its execution budget ({timeout:?})"),
        Fault::Memory => "ran out of memory".to_owned(),
    };
    let consequence = match standing {
        Standing::Unlimited => String::new(),
        Standing::Warned { faults, allowed } => {
            format!("; fault {faults} of {allowed} before it is quarantined")
        }
        Standing::Quarantined { faults } => format!(
            "; it faulted {faults} times and is quarantined: its handlers and timers no \
             longer run until it is reloaded (unload it to free its memory)"
        ),
    };
    format!("{what} of script `{id}` {cause}{consequence}")
}

/// Runs the Promise jobs the script just queued, inside its slice: the job queue belongs
/// to the runtime, not to a context, so jobs left for the end of the operation would run
/// under a budget no script is charged for (a handler that only does
/// `Promise.resolve().then(() => { while (true) {} })` would then cost a full timeout on
/// every emission and never be quarantined), and jobs of one script would run while the
/// next script's slice is the one that is open. Returns the first error a job threw.
fn drain_jobs(ctx: &Ctx<'_>) -> Result<(), EngineError> {
    // SAFETY: `ctx` is a live context of its runtime.
    let runtime = unsafe { qjs::JS_GetRuntime(ctx.as_raw().as_ptr()) };
    // SAFETY: `runtime` is alive. Most slices queue no job, and asking is cheaper than
    // running the loop below once to be told there is none.
    if !unsafe { qjs::JS_IsJobPending(runtime) } {
        return Ok(());
    }
    let mut first = None;
    while let Some(outcome) = run_next_job(runtime) {
        if let Err(error) = outcome {
            first.get_or_insert(error);
        }
    }
    first.map_or(Ok(()), Err)
}

/// Runs one pending job of `runtime`: `None` when there is none, else whether it threw.
fn run_next_job(runtime: *mut qjs::JSRuntime) -> Option<Result<(), EngineError>> {
    let mut job_context: *mut qjs::JSContext = ptr::null_mut();
    // SAFETY: `runtime` is alive, and the call writes the context the job ran in.
    let status = unsafe { qjs::JS_ExecutePendingJob(runtime, &mut job_context) };
    match status {
        0 => None,
        1.. => Some(Ok(())),
        _ => Some(Err(job_error(job_context))),
    }
}

/// The exception a job that threw left pending on the context it ran in, cleared.
fn job_error(job_context: *mut qjs::JSContext) -> EngineError {
    let Some(job_context) = NonNull::new(job_context) else {
        return EngineError::Execution {
            details: "a Promise job failed".to_owned(),
        };
    };
    // SAFETY: the context is the one the job ran in, borrowed from the runtime; `from_raw`
    // takes a reference of its own, released when `job` drops. The exception is pending on
    // it, and `pending_exception` clears it.
    let job = unsafe { Ctx::from_raw(job_context) };
    pending_exception(&job)
}
