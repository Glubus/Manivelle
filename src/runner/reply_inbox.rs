//! The runtime state of async host calls: settlements queued by resolvers, and the
//! reply conversion they carry, taken by the engine thread.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};

use rquickjs::{Ctx, Result as JsResult, Value as JsValue};

use crate::error::EngineError;

/// The output of one async host call, converted to JavaScript on the engine thread.
pub(crate) trait ReplyValue: Send {
    /// The value the script's Promise resolves to.
    fn into_js<'js>(self: Box<Self>, ctx: &Ctx<'js>) -> JsResult<JsValue<'js>>;
}

/// Turns a handler's output into a reply, validating it first when the registry
/// validates outputs.
pub(crate) type EncodeReply<T> =
    Arc<dyn Fn(T) -> Result<Box<dyn ReplyValue>, EngineError> + Send + Sync>;

/// How one async host call ended.
pub(crate) enum Settlement {
    /// The handler resolved it.
    Resolved(Box<dyn ReplyValue>),
    /// The handler rejected it, or its output failed to encode or validate; the
    /// message of the script's `Error`.
    Rejected(String),
    /// The resolver was dropped without settling.
    Dropped,
}

/// Settlements of one script version's async host calls: resolvers queue them from any
/// thread, and the engine thread takes them when it polls the version's
/// [`HostPromises`](crate::runner::host_promises::HostPromises).
///
/// Every call queues at most one settlement, so the queue never holds more entries than
/// the calls started.
pub(crate) struct ReplyInbox {
    state: Mutex<InboxState>,
    /// Raised whenever a settlement is queued, so the engine can tell that no reply
    /// waits anywhere without looking at every script.
    wake: Arc<AtomicBool>,
}

impl Default for ReplyInbox {
    fn default() -> Self {
        Self::new(Arc::default())
    }
}

#[derive(Default)]
struct InboxState {
    cancelled: bool,
    ready: VecDeque<(u64, Settlement)>,
}

impl ReplyInbox {
    pub(crate) fn new(wake: Arc<AtomicBool>) -> Self {
        Self {
            state: Mutex::default(),
            wake,
        }
    }

    /// Whether a settlement is queued.
    pub(crate) fn has_ready(&self) -> bool {
        self.lock().ready.front().is_some()
    }

    pub(crate) fn is_cancelled(&self) -> bool {
        self.lock().cancelled
    }

    /// The oldest queued settlement, with the id of its call.
    pub(crate) fn pop(&self) -> Option<(u64, Settlement)> {
        self.lock().ready.pop_front()
    }

    /// Refuses every later settlement and returns the queued ones, for the caller to
    /// drop once the lock is released.
    pub(crate) fn cancel(&self) -> VecDeque<(u64, Settlement)> {
        let mut state = self.lock();
        state.cancelled = true;
        std::mem::take(&mut state.ready)
    }

    /// Queues `settlement` of call `id`, or hands it back once cancelled so that it drops
    /// after the lock is released: dropping a host value may run arbitrary code.
    pub(crate) fn push(&self, id: u64, settlement: Settlement) -> Result<(), Settlement> {
        let mut state = self.lock();
        if state.cancelled {
            return Err(settlement);
        }
        state.ready.push_back((id, settlement));
        drop(state);
        self.wake.store(true, Ordering::Release);
        Ok(())
    }

    /// Each mutation is a single flag write, push, pop or take, so the state stays
    /// consistent even if a panic poisoned the lock.
    fn lock(&self) -> MutexGuard<'_, InboxState> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
