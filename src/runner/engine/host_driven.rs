//! Work the host drives: timers, Promise pumping and deferred calls and requests.

use std::ops::Range;
use std::sync::atomic::Ordering;
use std::time::Duration;

use rquickjs::{CatchResultExt, Ctx, Value as JsValue};

use super::invoke::invoke;
use crate::contract::{JsArgs, JsDecode, JsEncode};
use crate::error::EngineError;
use crate::types::ScriptId;

use super::super::errors::caught_js_error;
use super::super::tasks::{PendingCall, RequestGuard, RequestResults, ScriptTask};
use super::super::timers::run_due_timers;
use super::{Engine, EngineScript};

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
        let now = self.scheduling.timer_clock.advance(elapsed);
        if now < self.scheduling.earliest_timer.get() {
            // No script has a timer due yet: nothing to look at, in any context.
            return Ok(0);
        }
        let _budget = self.budget();
        let mut pass = TimerPass {
            engine: self,
            now,
            fired: 0,
            first_error: None,
            stopped: false,
        };
        pass.visit();
        self.refresh_earliest_timer();
        pass.finish()
    }

    /// What the scripts report now, exactly: the next `advance_timers` looks at them only
    /// once the clock gets there. A quarantined script's timers never fire, so they do not
    /// count.
    fn refresh_earliest_timer(&self) {
        self.scheduling.earliest_timer.set(
            self.scripts
                .values()
                .filter(|script| !script.quarantined.get())
                .map(|script| script.signals.next_timer.due())
                .fold(f64::INFINITY, f64::min),
        );
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
        self.run_in_script(script, |ctx| {
            let export = script.export(&self.shared.export_atoms, &ctx, script_id, function)?;
            let returned = invoke(&ctx, &export, &args)?;
            // SAFETY: `returned` is an owned value of `ctx`, which the wrapper releases.
            let returned = unsafe { JsValue::from_raw(ctx.clone(), returned) };
            let handle = PendingCall::new();
            if let Some(task) = ScriptTask::observe(&ctx, returned, &handle)? {
                script.tasks.borrow_mut().push(task);
                self.scheduling
                    .active_tasks
                    .set(self.scheduling.active_tasks.get() + 1);
            }
            Ok(handle)
        })
    }

    /// Delivers queued host replies on the engine thread and drains their Promise
    /// jobs under one execution budget. The host clock is not advanced. A call with no
    /// answer queued, which is most frames, is one atomic read however many scripts
    /// are loaded; only a script with an answer waiting is entered.
    pub fn pump(&self) -> Result<(), EngineError> {
        if !self.scheduling.host_wake.swap(false, Ordering::AcqRel) {
            // No resolver queued an answer since the last pump.
            return Ok(());
        }
        let _budget = self.budget();
        let mut first_error = None;
        let mut stopped = false;
        for (id, script) in &self.scripts {
            if self.execution.interrupt_requested() {
                stopped = true;
                break;
            }
            if let Err(error) = self.pump_script(id, script) {
                first_error.get_or_insert(error);
            }
        }
        if stopped || first_error.is_some() {
            // Answers may still wait: the next pump looks again.
            self.scheduling.host_wake.store(true, Ordering::Release);
        }
        self.begin_settle_slice();
        self.attribute_interrupt(self.settle(first_error.map_or(Ok(()), Err)))
    }

    /// Delivers the host answers waiting for one script, under a slice of its own.
    fn pump_script(&self, id: &str, script: &EngineScript) -> Result<(), EngineError> {
        if script.quarantined.get() || !script.host_promises.has_ready() {
            return Ok(());
        }
        script.context.with(|ctx| {
            self.run_slice(
                &ctx,
                id,
                script,
                || "a host answer".to_owned(),
                || {
                    script
                        .host_promises
                        .poll(&ctx)
                        .catch(&ctx)
                        .map_err(caught_js_error)
                        .map(drop)
                },
            )
        })
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
        let audience = self.everyone_listening_to(event);
        let emission =
            self.run_audience(&audience, event, payload, true, |script_id, ctx, value| {
                let script = self.script(script_id)?;
                script
                    .request_guards
                    .borrow_mut()
                    .push(RequestGuard::new(&replies));
                if let Some(task) = RequestResults::observe(&replies, ctx, script_id, value)? {
                    script.tasks.borrow_mut().push(task);
                    self.scheduling
                        .active_tasks
                        .set(self.scheduling.active_tasks.get() + 1);
                }
                Ok(())
            });
        let delivery = self.first_failure(emission);
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
}

/// A timer pass in progress: which timers are due, and what happened so far.
struct TimerPass<'a> {
    engine: &'a Engine,
    now: f64,
    /// The scripts whose timers fired.
    fired: usize,
    /// The first error of a script, as the pass reports it.
    first_error: Option<EngineError>,
    /// Set once the host interrupts the pass: no timer fires after that.
    stopped: bool,
}

impl TimerPass<'_> {
    /// Fires the due timers, one context visit per run of scripts that share a context, as
    /// `emit` does.
    fn visit(&mut self) {
        let engine = self.engine;
        let count = engine.scripts.len();
        let mut start = 0;
        while start < count && !self.stopped {
            let Some((_, first)) = engine.scripts.get_index(start) else {
                // Unreachable while the scripts change only through `&mut self`: skip the
                // position rather than the rest of the pass.
                start += 1;
                continue;
            };
            let run = engine.run_len(first, start + 1..count);
            if (start..start + run).any(|position| self.is_due(position)) {
                self.fire_run(first, start..start + run);
            }
            start += run;
        }
    }

    /// Whether the script at `position` has a timer due. A quarantined script's never is.
    fn is_due(&self, position: usize) -> bool {
        self.engine
            .scripts
            .get_index(position)
            .is_some_and(|(_, script)| {
                !script.quarantined.get() && script.signals.next_timer.is_due(self.now)
            })
    }

    fn fire_run(&mut self, first: &EngineScript, run: Range<usize>) {
        first.context.with(|ctx| {
            for position in run {
                if self.is_due(position) {
                    self.fire(position, &ctx);
                }
                if self.stopped {
                    break;
                }
            }
        });
    }

    /// Fires the due timers of the script at `position`, under a slice of its own.
    fn fire(&mut self, position: usize, ctx: &Ctx<'_>) {
        let engine = self.engine;
        let now = self.now;
        let Some((id, script)) = engine.scripts.get_index(position) else {
            return;
        };
        self.fired += 1;
        let outcome = engine.run_slice(
            ctx,
            id,
            script,
            || "a timer".to_owned(),
            || run_due_timers(ctx, &script.hooks.timers_run, now),
        );
        if let Err(error) = outcome {
            self.first_error.get_or_insert(error);
        }
        self.stopped = engine.execution.interrupt_requested();
    }

    /// Settles what the timers left, and reports the pass's result.
    fn finish(self) -> Result<usize, EngineError> {
        if self.fired == 0 {
            // No JavaScript ran, so there is no Promise job to settle.
            return Ok(0);
        }
        // The work left to settle runs under a budget of its own.
        self.engine.begin_settle_slice();
        self.engine.attribute_interrupt(
            self.engine
                .settle(self.first_error.map_or(Ok(self.fired), Err)),
        )
    }
}
