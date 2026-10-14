//! Scripts that run out of their budget while the host delivers to them: each script
//! of an emission, a timer pass or a pump has a budget of its own, and one that keeps
//! running out of it is quarantined.

use super::super::errors::OUT_OF_MEMORY;
use super::dispatch::NULL_EXCEPTION;
use super::{Engine, EngineScript};
use crate::error::EngineError;

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

    /// Starts the slice of the next script of a multi-script operation: its own budget.
    #[inline]
    pub(super) fn begin_slice(&self) {
        self.execution.restart_slice();
    }

    /// Ends the slice of `script`, which delivered with `outcome`: when it ran out of its
    /// time, or failed for lack of memory, that is a fault, and the error names the
    /// script (`what` says what was delivered to it). Any other outcome is returned as it
    /// is. The caller records the error and goes on to the next script unless the host
    /// asked to stop.
    pub(super) fn judge_slice(
        &self,
        id: &str,
        script: &EngineScript,
        outcome: Result<(), EngineError>,
        what: impl FnOnce() -> String,
    ) -> Result<(), EngineError> {
        let expired = self.execution.slice_expired();
        // An allocation failure is worded `out of memory`; the null or empty exception
        // QuickJS throws when it fails while handling one is only trusted near the limit.
        let out_of_memory = !expired
            && matches!(&outcome, Err(EngineError::Execution { details })
                if details.starts_with(OUT_OF_MEMORY) || self.is_out_of_memory(details));
        if !expired && !out_of_memory {
            return outcome;
        }
        let cause = if expired {
            format!(
                "ran out of its execution budget ({:?})",
                self.execution_timeout
            )
        } else {
            "ran out of memory".to_owned()
        };
        let faults = script.faults.get() + 1;
        script.faults.set(faults);
        let quarantine = self.max_script_faults != 0 && faults >= self.max_script_faults;
        let consequence = if quarantine {
            if !script.quarantined.replace(true) {
                self.event_index.mark_stale();
                script.host_promises.cancel();
            }
            format!(
                "; it faulted {faults} times and is quarantined: its handlers and timers no \
                 longer run until it is reloaded (unload it to free its memory)"
            )
        } else if self.max_script_faults != 0 {
            format!(
                "; fault {faults} of {} before it is quarantined",
                self.max_script_faults
            )
        } else {
            String::new()
        };
        Err(EngineError::Execution {
            details: format!("{} of script `{id}` {cause}{consequence}", what()),
        })
    }

    /// Whether an error message says the engine ran out of memory, as the engine words it
    /// when it is near its limit.
    pub(super) fn is_out_of_memory(&self, details: &str) -> bool {
        if !(details.is_empty()
            || details.starts_with(NULL_EXCEPTION)
            || details.starts_with(OUT_OF_MEMORY))
        {
            return false;
        }
        let stats = self.memory_stats();
        stats.malloc_limit_bytes != 0
            && stats.malloc_size_bytes * 10 >= stats.malloc_limit_bytes * 9
    }
}
