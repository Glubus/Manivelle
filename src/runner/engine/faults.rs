//! Scripts that run out of their budget while the host delivers to them: each script
//! of an emission, a timer pass or a pump has a budget of its own (a *slice*), and one that
//! keeps running out of it is quarantined.

use std::mem;
use std::ptr::{self, NonNull};

use rquickjs::{Ctx, qjs};

use super::super::errors::pending_exception;
use super::super::quota::ChargeGuard;
use super::dispatch::NULL_EXCEPTION;
use super::{Engine, EngineScript};
use crate::error::{EngineError, FaultStanding, ScriptFault};
use crate::types::ScriptId;

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

    /// Forgets the faults counted against a script that is not quarantined, so that its
    /// next one is its first again. Faults never expire on their own: a host that
    /// forgives a script for a slow frame an hour ago calls this where it sees fit, such
    /// as between levels. A quarantined script stays quarantined, since its timers and
    /// pending host answers were dropped with its handlers: reload it. Fails when the
    /// script is not loaded.
    pub fn clear_faults(&self, script_id: &str) -> Result<(), EngineError> {
        let script = self.script(script_id)?;
        if !script.quarantined.get() {
            script.faults.set(0);
        }
        Ok(())
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
        id: &ScriptId,
        script: &EngineScript,
        what: impl FnOnce() -> String,
        body: impl FnOnce() -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        let _charge = self.charge_to(script);
        self.execution.restart_slice();
        let outcome = body();
        let jobs = drain_jobs(ctx);
        self.judge_slice(ctx, id, script, outcome.and(jobs), what)
    }

    /// Charges the memory the script allocates, until the guard drops, to its quota; `None`
    /// when there is no quota.
    pub(super) fn charge_to(&self, script: &EngineScript) -> Option<ChargeGuard<'_>> {
        let (quota, owner) = (self.faults.quota.as_ref()?, script.owner.as_ref()?);
        Some(quota.charge_to(owner))
    }

    /// `result` with a failure for lack of memory named for what it is when the script
    /// went over its own quota, for the operations that are not slices (a call): a slice
    /// also counts the fault, a call does not.
    pub(super) fn explain_quota<T>(
        &self,
        id: &ScriptId,
        script: &EngineScript,
        what: impl FnOnce() -> String,
        result: Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        let (Err(error), Some(quota), Some(owner)) = (&result, &self.faults.quota, &script.owner)
        else {
            return result;
        };
        if !looks_like_exhaustion(error) || !owner.take_refused() {
            return result;
        }
        Err(EngineError::Fault {
            script_id: id.clone(),
            what: what(),
            fault: ScriptFault::Quota {
                limit_bytes: quota.limit(),
            },
            standing: FaultStanding::Unlimited,
        })
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
        id: &ScriptId,
        script: &EngineScript,
        outcome: Result<(), EngineError>,
        what: impl FnOnce() -> String,
    ) -> Result<(), EngineError> {
        let Some(fault) = self.fault_of(ctx, script, &outcome) else {
            return outcome;
        };
        Err(EngineError::Fault {
            script_id: id.clone(),
            what: what(),
            fault,
            standing: self.count_fault(script),
        })
    }

    /// The fault, if any, that the slice that just ended with `outcome` committed.
    ///
    /// The memory pool is shared, so a script that fails to allocate may be the victim of
    /// another one that holds the memory. Only the first script to fail for lack of memory
    /// in an operation is counted: those after it are reported, but not faulted.
    fn fault_of(
        &self,
        ctx: &Ctx<'_>,
        script: &EngineScript,
        outcome: &Result<(), EngineError>,
    ) -> Option<ScriptFault> {
        if self.execution.slice_expired() {
            return Some(ScriptFault::Budget {
                timeout: self.execution_timeout,
            });
        }
        let error = outcome.as_ref().err()?;
        if !looks_like_exhaustion(error) {
            return None;
        }
        // A script over its own quota is the culprit whatever the pool holds.
        if (script.owner.as_ref()).is_some_and(|owner| owner.take_refused()) {
            return (self.faults.quota.as_ref()).map(|quota| ScriptFault::Quota {
                limit_bytes: quota.limit(),
            });
        }
        let ran_out_of_memory = matches!(error, EngineError::OutOfMemory) || pool_nearly_full(ctx);
        (ran_out_of_memory && !self.faults.memory_fault_seen.replace(true))
            .then_some(ScriptFault::Memory)
    }

    /// Counts a fault against `script`, quarantining it at the limit.
    fn count_fault(&self, script: &EngineScript) -> FaultStanding {
        let faults = script.faults.get() + 1;
        script.faults.set(faults);
        let allowed = self.faults.max_script_faults;
        if allowed == 0 {
            return FaultStanding::Unlimited;
        }
        if faults < allowed {
            return FaultStanding::Warned { faults, allowed };
        }
        self.quarantine(script);
        FaultStanding::Quarantined { faults }
    }

    /// Takes `script` out of the event index, its timers and the pump.
    fn quarantine(&self, script: &EngineScript) {
        if !script.quarantined.replace(true) {
            self.event_index.mark_stale();
            script.host_promises.cancel();
        }
    }

    /// Whether `error` is QuickJS failing to allocate without saying so, while the engine
    /// is near its limit. Takes the runtime lock: not for use inside a context.
    pub(super) fn is_silent_out_of_memory(&self, error: &EngineError) -> bool {
        if !matches!(error, EngineError::Execution { .. }) || !looks_like_exhaustion(error) {
            return false;
        }
        let stats = self.memory_stats();
        near_limit(
            i128::from(stats.malloc_size_bytes),
            i128::from(stats.malloc_limit_bytes),
        )
    }
}

/// Whether an error is an out of memory, or what QuickJS gives when an allocation fails
/// while it was already handling a failure: nothing, or a thrown `null`. Those two are
/// only a hint, to confirm against how full the memory is.
fn looks_like_exhaustion(error: &EngineError) -> bool {
    match error {
        EngineError::OutOfMemory => true,
        EngineError::Execution { details } => {
            details.is_empty() || details.starts_with(NULL_EXCEPTION)
        }
        _ => false,
    }
}

/// Whether `used` is at least nine tenths of `limit`; a zero limit is no limit at all.
fn near_limit(used: i128, limit: i128) -> bool {
    limit != 0 && used * 10 >= limit * 9
}

/// Whether the engine's memory pool, measured from `ctx`, is at least nine tenths full.
/// Reads the runtime directly: this runs inside a context, where `memory_stats` would take
/// the runtime lock a second time.
fn pool_nearly_full(ctx: &Ctx<'_>) -> bool {
    // SAFETY: `JSMemoryUsage` is a plain struct of integers, for which all zeroes is valid.
    let mut usage: qjs::JSMemoryUsage = unsafe { mem::zeroed() };
    // SAFETY: `ctx` is a live context, so its runtime is alive, and `usage` is writable.
    unsafe { qjs::JS_ComputeMemoryUsage(qjs::JS_GetRuntime(ctx.as_raw().as_ptr()), &mut usage) };
    near_limit(
        i128::from(usage.malloc_size),
        i128::from(usage.malloc_limit),
    )
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
    run_jobs(|| run_next_job(runtime))
}

/// Runs Promise jobs one after another until `next` reports there is none, with `None`,
/// or with `Some` of whether that job threw. Every job runs even after one throws, so
/// none is left for the next operation; the first error is returned.
pub(super) fn run_jobs(
    mut next: impl FnMut() -> Option<Result<(), EngineError>>,
) -> Result<(), EngineError> {
    let mut first = None;
    while let Some(outcome) = next() {
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
