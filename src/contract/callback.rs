//! Host callback contract traits.

use super::HostContract;

/// Host callback contract: an event the host emits to the scripts' `ctx.on` handlers.
///
/// Fire-and-forget: the host emits the event and does not read any handler's result. A
/// callback whose handlers return a value is a [`HostRequest`] instead.
pub trait HostCallback: HostContract {
    /// Callback payload type.
    type Payload;
}

/// Host request: a [`HostCallback`] whose handlers answer.
/// [`Engine::request`](crate::Engine::request) returns what each handler returns. Register
/// it with [`request`](crate::InMemoryHostContractRegistry::request) so the generated
/// TypeScript types its handlers as returning `Reply`.
///
/// It is the only callback kind that carries a reply type: a plain [`HostCallback`] has none.
pub trait HostRequest: HostCallback {
    /// What one handler returns, or what the Promise an `async` handler returns
    /// resolves to.
    type Reply;
}
