//! Script timers (`setTimeout`, `setInterval`) on a clock the host advances.

use std::cell::Cell;
use std::rc::Rc;
use std::time::Duration;

use rquickjs::{Ctx, Function, qjs};

use super::errors::js_error;
use super::retained::{Retained, release_value};
use crate::error::EngineError;

/// Engine time in milliseconds, advanced only by
/// [`Engine::advance_timers`](crate::Engine::advance_timers); starts at 0.
#[derive(Clone, Default)]
pub(super) struct TimerClock(Rc<Cell<f64>>);

impl TimerClock {
    /// Moves the clock forward by `elapsed`; returns the new time.
    pub(super) fn advance(&self, elapsed: Duration) -> f64 {
        let now = self.0.get() + elapsed.as_secs_f64() * 1000.0;
        self.0.set(now);
        now
    }
}

/// Due time of a context's earliest timer, so the engine enters only contexts with a
/// timer to fire.
#[derive(Clone)]
pub(super) struct NextTimer(Rc<Cell<f64>>);

impl Default for NextTimer {
    fn default() -> Self {
        Self(Rc::new(Cell::new(f64::INFINITY)))
    }
}

impl NextTimer {
    pub(super) fn is_due(&self, now: f64) -> bool {
        self.0.get() <= now
    }

    pub(super) fn due(&self) -> f64 {
        self.0.get()
    }
}

/// A time no later than the earliest due time of any script, so a call that moves the
/// clock to a time before it knows no timer is due without asking every script. It only
/// ever errs early (a script's timer was cleared or fired, its due time moved later):
/// the engine then looks at every script and sets it exactly.
#[derive(Clone)]
pub(super) struct EarliestDue(Rc<Cell<f64>>);

impl Default for EarliestDue {
    fn default() -> Self {
        Self(Rc::new(Cell::new(f64::INFINITY)))
    }
}

impl EarliestDue {
    pub(super) fn get(&self) -> f64 {
        self.0.get()
    }

    pub(super) fn set(&self, due: f64) {
        self.0.set(due);
    }

    fn lower(&self, due: f64) {
        if due < self.0.get() {
            self.0.set(due);
        }
    }
}

/// The clock and scheduling hooks the prelude builds a script's timer functions on:
/// `now` reads the engine clock, `schedule` records the due time of the script's
/// earliest timer.
pub(super) struct TimerHooks<'js> {
    pub(super) now: Function<'js>,
    pub(super) schedule: Function<'js>,
}

pub(super) fn timer_hooks<'js>(
    ctx: &Ctx<'js>,
    clock: &TimerClock,
    next: &NextTimer,
    earliest: &EarliestDue,
) -> Result<TimerHooks<'js>, EngineError> {
    let clock = Rc::clone(&clock.0);
    let next = Rc::clone(&next.0);
    let earliest = earliest.clone();
    Ok(TimerHooks {
        now: Function::new(ctx.clone(), move || clock.get()).map_err(js_error)?,
        schedule: Function::new(ctx.clone(), move |due: f64| {
            next.set(due);
            earliest.lower(due);
        })
        .map_err(js_error)?,
    })
}

/// Fires a script's timers due at `now`, through the `run` function its prelude built.
///
/// The function is only borrowed for the call, as a handler is in
/// [`deliver`](super::events::deliver): `Function::call` would take and release several
/// references to the context and the value.
pub(super) fn run_due_timers(ctx: &Ctx<'_>, run: &Retained, now: f64) -> Result<(), EngineError> {
    // A number owns no reference, so there is nothing to release for the argument.
    let mut arguments = [qjs::JS_NewFloat64(now)];
    let returned = run.call(ctx, &mut arguments)?;
    // SAFETY: `returned` is the owned value `call` gave back, which nothing else holds, and
    // it is not used again; the function returns `undefined`.
    unsafe { release_value(ctx, returned) };
    Ok(())
}
