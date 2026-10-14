//! One-shot delivery of an async host function's result to the script awaiting it.

use std::collections::VecDeque;
use std::fmt;
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
    fn push(&self, id: u64, settlement: Settlement) -> Result<(), Settlement> {
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

/// Settles the Promise a script received from an async host function.
///
/// A handler registered with
/// [`InMemoryHostContractRegistry::async_function_with`](crate::InMemoryHostContractRegistry::async_function_with)
/// or one of its siblings receives one resolver per call. The resolver is `Send`: keep
/// it, move it to another thread, and settle it once with [`Self::resolve`] or
/// [`Self::reject`]. Settling only queues the result; the script observes it when the
/// engine's thread next calls [`Engine::pump`](crate::Engine::pump), and no JavaScript
/// runs before then.
///
/// Dropping a resolver without settling it rejects the script's Promise. Replacing or
/// unloading the script version that made the call cancels its resolvers, including the
/// ones whose result is queued but not yet delivered: the result is discarded,
/// [`Self::is_cancelled`] turns `true` and settling returns [`EngineError::Cancelled`].
#[must_use = "dropping a resolver without settling it rejects the script's Promise"]
pub struct HostResolver<T> {
    inbox: Arc<ReplyInbox>,
    id: u64,
    encode: EncodeReply<T>,
    settled: bool,
}

impl<T> HostResolver<T> {
    pub(crate) fn new(inbox: Arc<ReplyInbox>, id: u64, encode: EncodeReply<T>) -> Self {
        Self {
            inbox,
            id,
            encode,
            settled: false,
        }
    }

    /// Resolves the script's Promise with `value`.
    ///
    /// The value is encoded, and validated when the registry validates outputs, before
    /// this returns; conversion to JavaScript waits for the engine thread.
    ///
    /// # Errors
    ///
    /// [`EngineError::Cancelled`] when the calling script version is gone: nothing will
    /// observe `value`. Any other error is why `value` failed to encode or validate;
    /// the script's Promise rejects with it instead.
    pub fn resolve(mut self, value: T) -> Result<(), EngineError> {
        if self.is_cancelled() {
            self.settled = true;
            return Err(EngineError::Cancelled);
        }
        let encoded = (self.encode)(value);
        self.settled = true;
        match encoded {
            Ok(reply) => self.send(Settlement::Resolved(reply)),
            Err(error) => {
                self.send(Settlement::Rejected(error.to_string()))?;
                Err(error)
            }
        }
    }

    /// Rejects the script's Promise with an `Error` whose message is `error`'s.
    ///
    /// # Errors
    ///
    /// [`EngineError::Cancelled`] when the calling script version is gone: nothing will
    /// observe the rejection.
    pub fn reject(mut self, error: EngineError) -> Result<(), EngineError> {
        self.settled = true;
        self.send(Settlement::Rejected(error.to_string()))
    }

    /// Whether the script version that made the call was replaced or unloaded, so that
    /// nothing will observe a result. A host can check it to abandon work early.
    #[must_use]
    pub fn is_cancelled(&self) -> bool {
        self.inbox.is_cancelled()
    }

    fn send(&self, settlement: Settlement) -> Result<(), EngineError> {
        self.inbox
            .push(self.id, settlement)
            .map_err(|_discarded| EngineError::Cancelled)
    }
}

impl<T> Drop for HostResolver<T> {
    /// Rejects the Promise of a call nobody settled, so the script never waits forever.
    fn drop(&mut self) {
        if !self.settled {
            let _cancelled = self.inbox.push(self.id, Settlement::Dropped);
        }
    }
}

impl<T> fmt::Debug for HostResolver<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("HostResolver")
            .field("call", &self.id)
            .field("cancelled", &self.is_cancelled())
            .finish_non_exhaustive()
    }
}
