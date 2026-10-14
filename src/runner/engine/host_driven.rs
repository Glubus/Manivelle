//! Work the host drives: timers, Promise pumping and deferred calls and requests.

use std::sync::atomic::Ordering;
use std::time::Duration;

use rquickjs::{CatchResultExt, Value as JsValue};

use super::invoke::invoke;
use crate::contract::{JsArgs, JsDecode, JsEncode};
use crate::error::EngineError;
use crate::types::ScriptId;

use super::super::errors::caught_js_error;
use super::super::tasks::{PendingCall, RequestGuard, RequestResults, ScriptTask};
use super::super::timers::run_due_timers;
use super::Engine;

impl Engine {
    /// Moves the engine clock forward by `elapsed` and fires the timers now due, script
    /// by script in load order, each script's in due order; returns the number of
    /// scripts that had timers to fire. Call it from the host loop, typically once per
    /// frame with the frame time; nothing else moves the clock, so timers are
    /// deterministic.
    ///
    /// A timer fires at most once per call: one a callback schedules waits for the
    /// next call even with a zero delay, and an interval late by several periods fires
    /// once, its next due time staying its previous one plus its delay. A throwing
    /// callback does not stop the others; the first error is returned. An interrupt or
    /// an exhausted execution budget does: the timers of the scripts not reached yet
    /// stay due for the next call. Timers belong to the script version that set them:
    /// a reload or unload drops them.
    ///
    /// A call that finds no timer due, which is most frames, is one comparison however
    /// many scripts are loaded.
    pub fn advance_timers(&self, elapsed: Duration) -> Result<usize, EngineError> {
        let now = self.timer_clock.advance(elapsed);
        if now < self.earliest_timer.get() {
            // No script has a timer due yet: nothing to look at, in any context.
            return Ok(0);
        }
        let _budget = self.budget();
        let mut fired = 0;
        let mut first_error = None;
        // Scripts that follow one another in load order and share a context are served
        // in one visit to it, as `emit` does.
        let count = self.scripts.len();
        let mut start = 0;
        let mut stopped = false;
        while start < count && !stopped {
            let Some((_, first)) = self.scripts.get_index(start) else {
                break;
            };
            let mut end = start + 1;
            while let Some((_, next)) = self.scripts.get_index(end)
                && first.shares_context(next)
            {
                end += 1;
            }
            let any_due = (start..end).any(|index| {
                self.scripts
                    .get_index(index)
                    .is_some_and(|(_, script)| script.signals.next_timer.is_due(now))
            });
            if any_due {
                first.context.with(|ctx| {
                    for index in start..end {
                        let Some((_, script)) = self.scripts.get_index(index) else {
                            continue;
                        };
                        if !script.signals.next_timer.is_due(now) {
                            continue;
                        }
                        fired += 1;
                        if let Err(error) = run_due_timers(&ctx, &script.hooks.timers_run, now) {
                            first_error.get_or_insert(error);
                        }
                        if self.execution.should_stop() {
                            stopped = true;
                            break;
                        }
                    }
                });
            }
            start = end;
        }
        // What the scripts report now, exactly: the next call looks at them only once the
        // clock gets there.
        self.earliest_timer.set(
            self.scripts
                .values()
                .map(|script| script.signals.next_timer.due())
                .fold(f64::INFINITY, f64::min),
        );
        if fired == 0 {
            // No JavaScript ran, so there is no Promise job to settle.
            return Ok(0);
        }
        self.attribute_interrupt(self.settle(first_error.map_or(Ok(fired), Err)))
    }

    /// Starts an exported function without requiring its Promise to settle in this
    /// operation. The returned handle completes when the engine next drains jobs;
    /// `pump` delivers host replies without advancing the timer clock.
    pub fn call_deferred<R: JsDecode + 'static>(
        &self,
        script_id: &str,
        function: &str,
        args: impl JsArgs,
    ) -> Result<PendingCall<R>, EngineError> {
        let script = self.script(script_id)?;
        let _budget = self.budget();
        let result = script.context.with(|ctx| {
            let export = script.export(&self.export_atoms, &ctx, script_id, function)?;
            let returned = invoke(&ctx, &export, &args)?;
            // SAFETY: `returned` is an owned value of `ctx`, which the wrapper releases.
            let returned = unsafe { JsValue::from_raw(ctx.clone(), returned) };
            let handle = PendingCall::new();
            if let Some(task) = ScriptTask::observe(&ctx, returned, &handle)? {
                script.tasks.borrow_mut().push(task);
                self.active_tasks.set(self.active_tasks.get() + 1);
            }
            Ok(handle)
        });
        self.attribute_interrupt(self.settle(result))
    }

    /// Delivers queued host replies on the engine thread and drains their Promise
    /// jobs under one execution budget. The host clock is not advanced. A call with no
    /// answer queued, which is most frames, is one atomic read however many scripts
    /// are loaded; only a script with an answer waiting is entered.
    pub fn pump(&self) -> Result<(), EngineError> {
        if !self.host_wake.swap(false, Ordering::AcqRel) {
            // No resolver queued an answer since the last pump.
            return Ok(());
        }
        let _budget = self.budget();
        let mut first_error = None;
        let mut stopped = false;
        for script in self.scripts.values() {
            if self.execution.interrupted() {
                stopped = true;
                break;
            }
            if !script.host_promises.has_ready() {
                continue;
            }
            let result = script.context.with(|ctx| {
                script
                    .host_promises
                    .poll(&ctx)
                    .catch(&ctx)
                    .map_err(caught_js_error)
            });
            if let Err(error) = result {
                first_error.get_or_insert(error);
            }
        }
        if stopped || first_error.is_some() {
            // Answers may still wait: the next pump looks again.
            self.host_wake.store(true, Ordering::Release);
        }
        self.attribute_interrupt(self.settle(first_error.map_or(Ok(()), Err)))
    }

    /// Starts a request whose handlers may reply in later host-driven operations.
    /// Completed replies retain script and registration order, not resolution order.
    /// A handler error is returned through the handle after all replies finish.
    pub fn request_deferred<P: JsEncode + ?Sized, R: JsDecode + 'static>(
        &self,
        event: &str,
        payload: &P,
    ) -> Result<PendingCall<Vec<(ScriptId, R)>>, EngineError> {
        let handle = PendingCall::new();
        let replies = RequestResults::new(handle.clone());
        let delivery = self.dispatch(event, payload, true, |script_id, ctx, value| {
            let script = self.script(script_id)?;
            script
                .request_guards
                .borrow_mut()
                .push(RequestGuard::new(&replies));
            if let Some(task) = RequestResults::observe(&replies, ctx, script_id, value)? {
                script.tasks.borrow_mut().push(task);
                self.active_tasks.set(self.active_tasks.get() + 1);
            }
            Ok(())
        });
        if let Err(error) = delivery {
            replies.borrow_mut().fail(error);
        }
        replies.borrow_mut().activate();
        for script in self.scripts.values() {
            script
                .request_guards
                .borrow_mut()
                .retain(RequestGuard::is_pending);
        }
        Ok(handle)
    }

    /// Polls the scripts' deferred tasks; one comparison when none is waiting.
    #[inline]
    pub(super) fn harvest_tasks(&self) {
        if self.active_tasks.get() != 0 {
            self.poll_tasks();
        }
    }

    fn poll_tasks(&self) {
        for script in self.scripts.values() {
            let mut tasks = script.tasks.borrow_mut();
            if !tasks.is_empty() {
                let before = tasks.len();
                script.context.with(|ctx| {
                    tasks.retain_mut(|task| !task.poll(&ctx, &self.rejections, &self.module_store));
                });
                self.active_tasks
                    .set(self.active_tasks.get() - (before - tasks.len()));
            }
            script
                .request_guards
                .borrow_mut()
                .retain(RequestGuard::is_pending);
        }
    }
}
