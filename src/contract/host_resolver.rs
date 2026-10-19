//! One-shot delivery of an async host function's result to the script awaiting it.

use std::fmt;
use std::sync::Arc;

use crate::error::EngineError;
use crate::runner::reply_inbox::{EncodeReply, ReplyInbox, Settlement};

/// Settles the Promise a script received from an async host function.
///
/// Not a [`HostRequest`](crate::HostRequest) reply: a request's handlers answer
/// synchronously through their return value, while a resolver settles the Promise of one
/// async function call later, from any thread.
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
