//! Cooperative JavaScript execution budget.

use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::time::{Duration, Instant};

/// Deadline value meaning "no operation is running, or it has no time limit".
const NO_DEADLINE: u64 = u64::MAX;

/// Deadline value meaning "an operation with a time limit runs and its clock has not
/// started": nothing has read the time yet, which costs about as much as the rest of
/// delivering a small event.
const PENDING: u64 = u64::MAX - 1;

/// Deadline and interrupt request read by the QuickJS interrupt handler.
///
/// A budget starts when the operation first reaches a QuickJS interrupt check or a host
/// function, whichever comes first, by setting the deadline, in nanoseconds after
/// `origin`, to the budget from that moment: the host functions' time counts against
/// it, and an operation that reaches neither (a short handler, a few thousand
/// interpreter steps) never reads the clock. QuickJS checks the deadline about every
/// ten thousand interpreter steps, so JavaScript is stopped at the first check after it
/// passes. Only atomics are shared, so an interrupt can be requested from any thread.
#[derive(Debug)]
pub(crate) struct ExecutionControl {
    origin: Instant,
    deadline: AtomicU64,
    /// The running operation's budget, in nanoseconds, while its clock is pending.
    budget: AtomicU64,
    interrupt_requested: AtomicBool,
    /// Set when an interrupt check of the running operation stopped it: the budget had
    /// run out or an interrupt was requested.
    stopped: AtomicBool,
}

impl Default for ExecutionControl {
    fn default() -> Self {
        Self {
            origin: Instant::now(),
            deadline: AtomicU64::new(NO_DEADLINE),
            budget: AtomicU64::new(0),
            interrupt_requested: AtomicBool::new(false),
            stopped: AtomicBool::new(false),
        }
    }
}

impl ExecutionControl {
    /// The QuickJS interrupt check: whether JavaScript must stop now. Once it says so,
    /// [`ExecutionControl::should_stop`] does too until the next operation starts.
    pub(crate) fn interrupted(&self) -> bool {
        self.start_clock();
        let stop = self.interrupt_requested() || self.budget_expired();
        if stop {
            self.stopped.store(true, Ordering::Release);
        }
        stop
    }

    /// Whether the running operation must stop entering JavaScript: an interrupt was
    /// requested, or an interrupt check found its budget used up. It reads no clock, so
    /// checking between two handlers costs two atomic loads.
    pub(crate) fn should_stop(&self) -> bool {
        self.interrupt_requested() || self.stopped.load(Ordering::Acquire)
    }

    /// Starts the clock of the running operation's budget if it has not started: from
    /// here on its time counts. Called at every interrupt check and when a host function
    /// is entered.
    pub(crate) fn start_clock(&self) {
        if self.deadline.load(Ordering::Acquire) != PENDING {
            return;
        }
        let deadline = self
            .elapsed_nanos()
            .checked_add(self.budget.load(Ordering::Acquire))
            .filter(|&deadline| deadline < PENDING)
            .unwrap_or(NO_DEADLINE);
        self.deadline.store(deadline, Ordering::Release);
    }

    /// Gives the next script of a multi-script operation (an emission, a timer pass, a
    /// pump) a budget of its own: the operation's budget again, its clock not started.
    /// An interrupt the host requested is not cleared, so it still stops the operation.
    /// Costs one atomic load when the running script has not started the clock.
    pub(crate) fn restart_slice(&self) {
        match self.deadline.load(Ordering::Acquire) {
            NO_DEADLINE | PENDING => {}
            _ if self.interrupt_requested() => {}
            _ => {
                self.stopped.store(false, Ordering::Release);
                self.deadline.store(PENDING, Ordering::Release);
            }
        }
    }

    /// Whether the budget ran out, rather than the host asking to stop: the script that
    /// ran since the last [`ExecutionControl::restart_slice`] used up its slice.
    pub(crate) fn slice_expired(&self) -> bool {
        self.stopped.load(Ordering::Acquire) && !self.interrupt_requested()
    }

    /// Asks the running operation to stop at its next interrupt check. Starting an
    /// operation clears the request, so a request made while none runs has no effect.
    pub(crate) fn request_interrupt(&self) {
        self.interrupt_requested.store(true, Ordering::Release);
    }

    /// Whether the current or last operation was asked to stop.
    pub(crate) fn interrupt_requested(&self) -> bool {
        self.interrupt_requested.load(Ordering::Acquire)
    }

    /// Whether the running operation has used up its budget. False when no
    /// operation runs, it has no time limit or its clock has not started.
    pub(crate) fn budget_expired(&self) -> bool {
        match self.deadline.load(Ordering::Acquire) {
            NO_DEADLINE | PENDING => false,
            deadline => self.elapsed_nanos() >= deadline,
        }
    }

    /// Begins an operation with a budget of `budget`, whose clock starts at its first
    /// interrupt check or host function; it ends when the returned guard drops.
    pub(crate) fn enter(&self, budget: Duration) -> ExecutionGuard<'_> {
        let nanos = u64::try_from(budget.as_nanos())
            .ok()
            .filter(|&nanos| nanos < PENDING);
        self.interrupt_requested.store(false, Ordering::Release);
        self.stopped.store(false, Ordering::Release);
        match nanos {
            Some(nanos) => {
                self.budget.store(nanos, Ordering::Release);
                self.deadline.store(PENDING, Ordering::Release);
            }
            None => self.deadline.store(NO_DEADLINE, Ordering::Release),
        }
        ExecutionGuard(self)
    }

    fn elapsed_nanos(&self) -> u64 {
        u64::try_from(self.origin.elapsed().as_nanos()).unwrap_or(NO_DEADLINE - 1)
    }
}

pub(crate) struct ExecutionGuard<'a>(&'a ExecutionControl);

impl Drop for ExecutionGuard<'_> {
    fn drop(&mut self) {
        self.0.deadline.store(NO_DEADLINE, Ordering::Release);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_zero_budget_is_expired_at_the_first_check() {
        let control = ExecutionControl::default();
        let _guard = control.enter(Duration::ZERO);

        assert!(!control.budget_expired(), "the clock has not started");
        assert!(control.interrupted());
        assert!(control.budget_expired());
        assert!(!control.interrupt_requested());
    }

    #[test]
    fn a_host_function_starts_the_clock_and_its_time_counts() {
        let control = ExecutionControl::default();
        let _guard = control.enter(Duration::from_millis(5));

        control.start_clock();
        std::thread::sleep(Duration::from_millis(20));

        assert!(control.budget_expired());
    }

    #[test]
    fn an_operation_that_never_checks_never_reads_the_clock() {
        let control = ExecutionControl::default();
        let _guard = control.enter(Duration::from_millis(1));
        std::thread::sleep(Duration::from_millis(10));

        assert!(!control.budget_expired());
    }

    #[test]
    fn an_unlimited_budget_never_expires() {
        let control = ExecutionControl::default();
        let _guard = control.enter(Duration::MAX);

        assert!(!control.interrupted());
    }

    #[test]
    fn a_budget_ends_with_its_guard() {
        let control = ExecutionControl::default();
        drop(control.enter(Duration::ZERO));

        assert!(!control.budget_expired());
        assert!(!control.interrupted());
    }

    #[test]
    fn entering_clears_an_interrupt_request_but_not_a_later_one() {
        let control = ExecutionControl::default();
        control.request_interrupt();
        let _guard = control.enter(Duration::MAX);
        assert!(!control.interrupted());

        control.request_interrupt();
        assert!(control.interrupted());
        assert!(!control.budget_expired());
    }

    #[test]
    fn handlers_stop_once_a_check_finds_the_budget_used_up_until_the_next_operation() {
        let control = ExecutionControl::default();
        let guard = control.enter(Duration::ZERO);
        assert!(
            !control.should_stop(),
            "no check has found the budget used up"
        );

        assert!(control.interrupted());
        assert!(control.should_stop());

        drop(guard);
        let _guard = control.enter(Duration::MAX);
        assert!(
            !control.should_stop(),
            "a new operation starts with its own budget"
        );
    }

    #[test]
    fn a_new_slice_gives_the_next_script_its_own_budget() {
        let control = ExecutionControl::default();
        let _guard = control.enter(Duration::ZERO);
        assert!(control.interrupted());
        assert!(control.slice_expired());

        control.restart_slice();

        assert!(!control.slice_expired());
        assert!(!control.should_stop());
        assert!(!control.budget_expired(), "its clock has not started");
    }

    #[test]
    fn a_host_interrupt_survives_a_new_slice() {
        let control = ExecutionControl::default();
        let _guard = control.enter(Duration::MAX);
        control.request_interrupt();
        assert!(control.interrupted());

        control.restart_slice();

        assert!(control.should_stop());
        assert!(
            !control.slice_expired(),
            "the host asked, the budget did not run out"
        );
    }
}
