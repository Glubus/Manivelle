//! The Promises scripts await on async host functions, settled from Rust through
//! [`HostResolver`]s and delivered to JavaScript only when the engine polls.

use std::cell::{Cell, RefCell};
use std::rc::{Rc, Weak};
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use rquickjs::{
    Ctx, Error as JsError, Exception, Function, Persistent, Promise, Result as JsResult,
    Value as JsValue,
};
use rustc_hash::FxHashMap;

use crate::contract::{EncodeReply, HostResolver, ReplyInbox, Settlement};
use crate::error::EngineError;

/// The async host calls of one script version, from start to settlement.
///
/// The version owns it. The native functions its scripts call hold a
/// [`WeakHostPromises`] only, so no JavaScript object keeps the scope, or the Promise
/// functions it holds, alive: the QuickJS cycle collector cannot see references held
/// by Rust, and a strong one would form a cycle it never frees.
///
/// Resolvers queue settlements from any thread; [`Self::poll`] turns them into Promise
/// settlements on the engine thread. [`Self::cancel`], and dropping, retire the scope:
/// its functions reject every new call with [`EngineError::Cancelled`], its resolvers report
/// cancelled and queue nothing, and the Promises still waiting are released unsettled.
#[derive(Default)]
pub(crate) struct HostPromises {
    scope: Rc<Scope>,
}

/// What the native functions of a script version hold: calls start only while the
/// version's [`HostPromises`] lives and is not cancelled.
#[derive(Clone)]
pub(crate) struct WeakHostPromises(Weak<Scope>);

#[derive(Default)]
struct Scope {
    inbox: Arc<ReplyInbox>,
    pending: RefCell<FxHashMap<u64, PendingPromise>>,
    next_id: Cell<u64>,
}

/// The functions settling one waiting Promise.
struct PendingPromise {
    contract: &'static str,
    resolve: Persistent<Function<'static>>,
    reject: Persistent<Function<'static>>,
}

impl HostPromises {
    /// A scope whose resolvers raise `wake` each time they queue an answer.
    pub(crate) fn with_wake(wake: Arc<AtomicBool>) -> Self {
        Self {
            scope: Rc::new(Scope {
                inbox: Arc::new(ReplyInbox::new(wake)),
                pending: RefCell::default(),
                next_id: Cell::default(),
            }),
        }
    }

    /// Whether an answer waits to be settled, without entering the script's context.
    pub(crate) fn has_ready(&self) -> bool {
        self.scope.inbox.has_ready()
    }

    /// A handle for native functions that does not keep this scope alive.
    pub(crate) fn downgrade(&self) -> WeakHostPromises {
        WeakHostPromises(Rc::downgrade(&self.scope))
    }

    /// Settles the Promise of every call whose resolver answered, oldest answer first,
    /// and returns how many it settled.
    ///
    /// Settling queues the Promises' reactions as jobs without running them, and runs
    /// no JavaScript. A call whose resolver was dropped unsettled rejects. Answers to
    /// calls that already rejected, because their handler failed, are skipped. After
    /// [`Self::cancel`] nothing is queued, so this settles nothing.
    ///
    /// # Errors
    ///
    /// A failure to call a Promise function; the answers after it stay queued for the
    /// next poll.
    pub(crate) fn poll(&self, ctx: &Ctx<'_>) -> JsResult<usize> {
        let mut settled = 0;
        while let Some((id, settlement)) = self.scope.inbox.pop() {
            let pending = self.scope.pending.borrow_mut().remove(&id);
            if let Some(pending) = pending {
                pending.settle(ctx, settlement)?;
                settled += 1;
            }
        }
        Ok(settled)
    }

    /// Retires the scope: new calls reject with [`EngineError::Cancelled`], resolvers report
    /// cancelled and queue nothing, queued answers are discarded, and the waiting
    /// Promises are released unsettled.
    ///
    /// Releasing frees JavaScript values, so the runtime must still be alive.
    pub(crate) fn cancel(&self) {
        let queued = self.scope.inbox.cancel();
        let pending = std::mem::take(&mut *self.scope.pending.borrow_mut());
        drop(queued);
        drop(pending);
    }
}

impl Drop for HostPromises {
    fn drop(&mut self) {
        self.cancel();
    }
}

impl WeakHostPromises {
    /// Starts one call of async host function `contract` and returns the Promise the
    /// script awaits.
    ///
    /// `input` converts the call's argument, then `handler` receives it with the
    /// [`HostResolver`] of the call, whose answer `encode` converts. The Promise
    /// rejects right away, without running `input` or `handler`, when the scope is
    /// cancelled or gone, and rejects right away when `input` or `handler` fails; a
    /// resolver that outlives its handler's failure settles nothing.
    ///
    /// # Errors
    ///
    /// A failure to create or reject the Promise.
    pub(crate) fn call<'js, I, T>(
        &self,
        ctx: &Ctx<'js>,
        contract: &'static str,
        encode: &EncodeReply<T>,
        input: impl FnOnce() -> JsResult<I>,
        handler: impl FnOnce(I, HostResolver<T>) -> Result<(), EngineError>,
    ) -> JsResult<JsValue<'js>> {
        let (promise, resolve, reject) = Promise::new(ctx)?;
        let Some(scope) = self.0.upgrade().filter(|scope| !scope.inbox.is_cancelled()) else {
            reject.call::<_, ()>((error_value(ctx, &EngineError::Cancelled.to_string())?,))?;
            return Ok(promise.into_value());
        };
        let input = match input() {
            Ok(input) => input,
            Err(error) => {
                reject.call::<_, ()>((rejection_reason(ctx, error)?,))?;
                return Ok(promise.into_value());
            }
        };
        let id = scope.insert(ctx, contract, resolve, reject.clone());
        let resolver = HostResolver::new(Arc::clone(&scope.inbox), id, Arc::clone(encode));
        if let Err(error) = handler(input, resolver) {
            let pending = scope.pending.borrow_mut().remove(&id);
            if pending.is_some() {
                reject.call::<_, ()>((error_value(ctx, &error.to_string())?,))?;
            }
        }
        Ok(promise.into_value())
    }
}

impl Scope {
    fn insert<'js>(
        &self,
        ctx: &Ctx<'js>,
        contract: &'static str,
        resolve: Function<'js>,
        reject: Function<'js>,
    ) -> u64 {
        let id = self.next_id.get();
        self.next_id.set(id + 1);
        let pending = PendingPromise {
            contract,
            resolve: Persistent::save(ctx, resolve),
            reject: Persistent::save(ctx, reject),
        };
        self.pending.borrow_mut().insert(id, pending);
        id
    }
}

impl PendingPromise {
    fn settle(self, ctx: &Ctx<'_>, settlement: Settlement) -> JsResult<()> {
        let reason = match settlement {
            Settlement::Resolved(reply) => match reply.into_js(ctx) {
                Ok(value) => return self.resolve.restore(ctx)?.call::<_, ()>((value,)),
                Err(error) => rejection_reason(ctx, error)?,
            },
            Settlement::Rejected(message) => error_value(ctx, &message)?,
            Settlement::Dropped => error_value(
                ctx,
                &format!(
                    "host function `{}` dropped its resolver without settling the call",
                    self.contract
                ),
            )?,
        };
        self.reject.restore(ctx)?.call::<_, ()>((reason,))
    }
}

/// The value a failed conversion rejects with: the exception it threw, or an `Error`
/// describing it.
fn rejection_reason<'js>(ctx: &Ctx<'js>, error: JsError) -> JsResult<JsValue<'js>> {
    match error {
        JsError::Exception => Ok(ctx.catch()),
        error => error_value(ctx, &error.to_string()),
    }
}

fn error_value<'js>(ctx: &Ctx<'js>, message: &str) -> JsResult<JsValue<'js>> {
    Exception::from_message(ctx.clone(), message).map(Exception::into_value)
}

#[cfg(test)]
mod tests {
    use std::sync::Arc;

    use rquickjs::{Context, Ctx, Promise, Result as JsResult, Runtime};

    use super::{HostPromises, WeakHostPromises};
    use crate::contract::{EncodeReply, HostResolver, ReplyValue};
    use crate::error::EngineError;

    struct Number(f64);

    impl ReplyValue for Number {
        fn into_js<'js>(self: Box<Self>, ctx: &Ctx<'js>) -> JsResult<rquickjs::Value<'js>> {
            Ok(rquickjs::Value::new_number(ctx.clone(), self.0))
        }
    }

    /// Rejects negative numbers, standing in for output validation.
    fn encode() -> EncodeReply<f64> {
        Arc::new(|value: f64| {
            if value < 0.0 {
                return Err(EngineError::ContractValidation {
                    contract_name: "test.score".to_owned(),
                    direction: "output",
                    details: "negative".to_owned(),
                });
            }
            Ok(Box::new(Number(value)) as Box<dyn ReplyValue>)
        })
    }

    /// Starts one call on `promises` and hands its resolver to the test.
    fn start<'js>(
        ctx: &Ctx<'js>,
        promises: &WeakHostPromises,
    ) -> (Promise<'js>, Option<HostResolver<f64>>) {
        let mut kept = None;
        let promise = promises
            .call(
                ctx,
                "test.score",
                &encode(),
                || Ok(()),
                |(), resolver| {
                    kept = Some(resolver);
                    Ok(())
                },
            )
            .expect("start call");
        (promise.into_promise().expect("a promise"), kept)
    }

    fn rejection_message<'js>(ctx: &Ctx<'js>, promise: &Promise<'js>) -> String {
        assert!(promise.result::<()>().expect("settled").is_err());
        let reason = ctx.catch();
        let reason = reason.as_object().expect("an error object");
        reason.get::<_, String>("message").expect("message")
    }

    fn with_context(test: impl FnOnce(Ctx<'_>)) {
        let runtime = Runtime::new().expect("runtime");
        let context = Context::full(&runtime).expect("context");
        context.with(test);
    }

    #[test]
    fn resolution_waits_for_poll_and_runs_no_job() {
        let runtime = Runtime::new().expect("runtime");
        let context = Context::full(&runtime).expect("context");
        let promises = HostPromises::default();
        context.with(|ctx| {
            let (promise, resolver) = start(&ctx, &promises.downgrade());
            ctx.globals()
                .set("pending", promise.clone())
                .expect("global");
            ctx.eval::<(), _>("globalThis.seen = []; pending.then(value => seen.push(value));")
                .expect("attach reaction");

            resolver
                .expect("resolver")
                .resolve(7.0)
                .expect("queue reply");
            assert!(promise.result::<f64>().is_none(), "resolving runs no JS");

            assert_eq!(promises.poll(&ctx).expect("poll"), 1);
            assert_eq!(
                promise.result::<f64>().expect("settled").expect("value"),
                7.0
            );
            let seen: Vec<f64> = ctx.eval("seen").expect("seen");
            assert!(seen.is_empty(), "poll drains no job");
            assert_eq!(promises.poll(&ctx).expect("poll again"), 0);
        });

        assert!(runtime.execute_pending_job().expect("run reaction"));
        let seen: Vec<f64> = context.with(|ctx| ctx.eval("seen").expect("seen"));
        assert_eq!(seen, [7.0], "the reaction was queued");
    }

    #[test]
    fn rejection_and_dropped_resolver_reject_the_promise() {
        with_context(|ctx| {
            let promises = HostPromises::default();
            let weak = promises.downgrade();
            let (rejected, resolver) = start(&ctx, &weak);
            resolver
                .expect("resolver")
                .reject(EngineError::Execution {
                    details: "no score".to_owned(),
                })
                .expect("queue rejection");
            let (dropped, resolver) = start(&ctx, &weak);
            drop(resolver);

            assert_eq!(promises.poll(&ctx).expect("poll"), 2);
            assert!(rejection_message(&ctx, &rejected).contains("no score"));
            assert!(rejection_message(&ctx, &dropped).contains("dropped its resolver"));
        });
    }

    #[test]
    fn invalid_output_rejects_the_promise_and_reports_to_the_host() {
        with_context(|ctx| {
            let promises = HostPromises::default();
            let (promise, resolver) = start(&ctx, &promises.downgrade());

            let error = resolver.expect("resolver").resolve(-1.0);

            assert!(
                matches!(error, Err(EngineError::ContractValidation { .. })),
                "{error:?}"
            );
            assert_eq!(promises.poll(&ctx).expect("poll"), 1);
            assert!(rejection_message(&ctx, &promise).contains("output validation failed"));
        });
    }

    #[test]
    fn failing_handler_rejects_at_once_and_its_resolver_settles_nothing() {
        with_context(|ctx| {
            let promises = HostPromises::default();
            let mut escaped = None;
            let promise = promises
                .downgrade()
                .call(
                    &ctx,
                    "test.score",
                    &encode(),
                    || Ok(()),
                    |(), resolver| {
                        escaped = Some(resolver);
                        Err(EngineError::Execution {
                            details: "busy".to_owned(),
                        })
                    },
                )
                .expect("start call")
                .into_promise()
                .expect("a promise");

            assert!(rejection_message(&ctx, &promise).contains("busy"));
            escaped.expect("resolver").resolve(1.0).expect("queued");
            assert_eq!(promises.poll(&ctx).expect("poll"), 0);
        });
    }

    #[test]
    fn cancel_discards_queued_replies_and_refuses_new_ones() {
        with_context(|ctx| {
            let promises = HostPromises::default();
            let weak = promises.downgrade();
            let (queued, first) = start(&ctx, &weak);
            let (waiting, second) = start(&ctx, &weak);
            first.expect("resolver").resolve(1.0).expect("queue reply");
            let second = second.expect("resolver");

            promises.cancel();

            assert!(second.is_cancelled(), "cancel reaches unsettled resolvers");
            assert!(matches!(second.resolve(2.0), Err(EngineError::Cancelled)));
            assert_eq!(
                promises.poll(&ctx).expect("poll"),
                0,
                "no callback after cancel"
            );
            assert!(queued.result::<f64>().is_none());
            assert!(waiting.result::<f64>().is_none());
            let (late, resolver) = start(&ctx, &weak);
            assert!(resolver.is_none(), "a cancelled scope runs no handler");
            assert!(rejection_message(&ctx, &late).contains(&EngineError::Cancelled.to_string()));
        });
    }

    #[test]
    fn dropped_scope_rejects_calls_and_cancels_resolvers() {
        with_context(|ctx| {
            let promises = HostPromises::default();
            let weak = promises.downgrade();
            let (_, resolver) = start(&ctx, &weak);
            let resolver = resolver.expect("resolver");

            drop(promises);

            assert!(resolver.is_cancelled());
            assert!(matches!(
                resolver.reject(EngineError::Cancelled),
                Err(EngineError::Cancelled)
            ));
            let (late, handler_ran) = start(&ctx, &weak);
            assert!(handler_ran.is_none(), "a dropped scope runs no handler");
            assert!(rejection_message(&ctx, &late).contains(&EngineError::Cancelled.to_string()));
        });
    }
}
