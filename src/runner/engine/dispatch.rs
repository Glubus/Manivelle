//! Calling exports and delivering events and requests, and the settling that ends
//! every operation.

use rquickjs::{CatchResultExt, CaughtError, Ctx, Value as JsValue};

use crate::contract::{JsArgs, JsDecode, JsEncode};
use crate::error::EngineError;

use super::super::errors::{OUT_OF_MEMORY, caught_js_error, in_typescript, js_error};
use super::super::events::deliver;
use super::super::host_fn;
use super::Engine;
use super::invoke::invoke;

/// How a thrown `null` is described: QuickJS throws it when it runs out of memory while
/// already handling an out of memory. Built without its assertions, the same failure
/// surfaces as an exception with no message at all.
const NULL_EXCEPTION: &str = "non-error exception: Null";

impl Engine {
    /// Calls one exported function. Arguments encode through [`JsArgs`] (a tuple, a
    /// `Vec` or a slice) and the result decodes through [`JsDecode`], natively on both
    /// sides; `call::<serde_json::Value>(id, name, vec![json])` keeps a JSON-shaped API.
    /// An `async` export resolves before its value is decoded.
    pub fn call<R: JsDecode>(
        &self,
        script_id: &str,
        function: &str,
        args: impl JsArgs,
    ) -> Result<R, EngineError> {
        let script = self.script(script_id)?;
        let _budget = self.budget();
        let result = script.context.with(|ctx| {
            let export = script.export(&self.export_atoms, &ctx, script_id, function)?;
            let returned = invoke(&ctx, &export, &args)?;
            // A number, a boolean or nothing owns no memory: read it where it is.
            if let Some(value) = R::decode_scalar(returned) {
                return Ok(value);
            }
            // SAFETY: `returned` is an owned value of `ctx`, which the wrapper releases.
            let returned = unsafe { JsValue::from_raw(ctx.clone(), returned) };
            let value = self.await_returned(&ctx, returned, || {
                format!("function `{function}` of script `{script_id}`")
            })?;
            R::decode_js(&ctx, value)
                .catch(&ctx)
                .map_err(caught_js_error)
        });
        self.attribute_interrupt(self.settle(result))
    }

    /// Delivers one event to every handler registered for it, script by script in load
    /// order; returns the number of scripts that had at least one handler. A throwing
    /// handler does not stop the others: every handler runs, then the first error is
    /// returned. An interrupt or an exhausted execution budget does stop them: no
    /// handler is entered after it. The payload is encoded once per script with
    /// handlers and shared by all of that script's handlers. Scripts that never
    /// registered a handler for `event` cost a set lookup.
    pub fn emit<P: JsEncode + ?Sized>(
        &self,
        event: &str,
        payload: &P,
    ) -> Result<usize, EngineError> {
        self.dispatch(event, payload, false, |_, _, _| Ok(()))
    }

    /// Delivers one event like [`Engine::emit`] and returns what every handler returned,
    /// with the id of its script, in delivery order: scripts in load order, handlers in
    /// registration order. An `async` handler's Promise is awaited first, running
    /// script Promise jobs only. Every handler runs even after one fails, a throw, a
    /// rejection or a reply that does not decode as `R`; the first error is returned.
    /// An interrupt or an exhausted execution budget stops delivery, as for `emit`.
    /// Register the event with
    /// [`request`](crate::InMemoryHostContractRegistry::request) so the
    /// generated TypeScript types the handlers' reply.
    pub fn request<P: JsEncode + ?Sized, R: JsDecode>(
        &self,
        event: &str,
        payload: &P,
    ) -> Result<Vec<(&str, R)>, EngineError> {
        let mut replies = Vec::new();
        self.dispatch(event, payload, true, |script_id, ctx, returned| {
            let value = self.await_returned(ctx, returned, || {
                format!("a `{event}` handler of script `{script_id}`")
            })?;
            let reply = R::decode_js(ctx, value)
                .catch(ctx)
                .map_err(caught_js_error)?;
            replies.push((script_id, reply));
            Ok(())
        })?;
        Ok(replies)
    }

    /// Runs every handler of `event` in load order, handing each returned value to
    /// `on_return`; see [`Engine::emit`].
    pub(super) fn dispatch<'a, P: JsEncode + ?Sized>(
        &'a self,
        event: &str,
        payload: &P,
        keep_results: bool,
        mut on_return: impl for<'js> FnMut(&'a str, &Ctx<'js>, JsValue<'js>) -> Result<(), EngineError>,
    ) -> Result<usize, EngineError> {
        // Only the scripts listening to `event` are visited, in load order.
        let Some(listeners) = self.event_index.listeners(
            event,
            self.scripts.values().map(|script| &script.signals.events),
        ) else {
            return Ok(0);
        };
        let _budget = self.budget();
        let mut delivered = 0;
        let mut first_error = None;
        // Listeners that follow one another and share a context (a group's) are served
        // in one visit to it: the context is entered once and the payload encoded once,
        // instead of once per script. The order is the load order.
        let mut start = 0;
        // Set once the operation is interrupted or out of budget: no handler is entered
        // after that, and the error is already recorded.
        let mut stopped = false;
        while start < listeners.len() && !stopped {
            let Some((first_id, first)) = self.scripts.get_index(listeners[start]) else {
                break;
            };
            let mut end = start + 1;
            while end < listeners.len()
                && let Some((_, next)) = self.scripts.get_index(listeners[end])
                && first.shares_context(next)
            {
                end += 1;
            }
            if end - start == 1 {
                // A context of its own: one look at the script's handlers decides
                // whether to enter it (an earlier handler may have removed them).
                if let Some(handlers) = first.signals.events.handlers(event) {
                    delivered += 1;
                    let outcome = first.context.with(|ctx| match payload.encode_js(&ctx) {
                        Ok(payload) => deliver(
                            &ctx,
                            &handlers,
                            &payload,
                            keep_results,
                            &self.execution,
                            |returned| on_return(first_id, &ctx, returned),
                        ),
                        Err(error) => Err(js_error(error)),
                    });
                    if let Err(error) = outcome {
                        first_error.get_or_insert(error);
                    }
                    stopped = self.execution.should_stop();
                }
                start = end;
                continue;
            }
            first.context.with(|ctx| match payload.encode_js(&ctx) {
                Ok(payload) => {
                    for &position in &listeners[start..end] {
                        let Some((script_id, script)) = self.scripts.get_index(position) else {
                            continue;
                        };
                        let Some(handlers) = script.signals.events.handlers(event) else {
                            continue;
                        };
                        delivered += 1;
                        let outcome = deliver(
                            &ctx,
                            &handlers,
                            &payload,
                            keep_results,
                            &self.execution,
                            |returned| on_return(script_id, &ctx, returned),
                        );
                        if let Err(error) = outcome {
                            first_error.get_or_insert(error);
                        }
                        if self.execution.should_stop() {
                            stopped = true;
                            break;
                        }
                    }
                }
                Err(error) => {
                    // Nothing to hand over: the scripts that listen fail together.
                    first_error.get_or_insert(js_error(error));
                    delivered += listeners[start..end]
                        .iter()
                        .filter(|&&position| {
                            self.scripts
                                .get_index(position)
                                .is_some_and(|(_, script)| script.signals.events.listens(event))
                        })
                        .count();
                }
            });
            start = end;
        }
        if delivered == 0 {
            // No JavaScript ran, so there is no Promise job to settle.
            return Ok(0);
        }
        self.attribute_interrupt(self.settle(first_error.map_or(Ok(delivered), Err)))
    }

    /// Awaits a Promise a script returned to the host, from an export or a request
    /// handler, by running the job queue; `what` names the function in the error of a
    /// Promise that cannot settle. Its rejection is the error, so it is not also
    /// reported as unhandled.
    fn await_returned<'js>(
        &self,
        ctx: &Ctx<'js>,
        returned: JsValue<'js>,
        what: impl FnOnce() -> String,
    ) -> Result<JsValue<'js>, EngineError> {
        let Some(promise) = returned.as_promise() else {
            return Ok(returned);
        };
        let settled = promise.finish::<JsValue<'_>>().catch(ctx);
        self.rejections.forget(ctx, &returned);
        settled.map_err(|error| match error {
            CaughtError::Error(rquickjs::Error::WouldBlock) => EngineError::Execution {
                details: format!(
                    "{} returned a Promise that never settles: only script Promise jobs run on an Engine",
                    what()
                ),
            },
            error => caught_js_error(error),
        })
    }

    /// Runs every job left in the queue, for all scripts, then reports in order: the
    /// operation's own error, a job that threw, a Promise rejection nobody handled,
    /// with its locations pointing at the TypeScript source. Runs even when the
    /// operation failed, so nothing leaks into the next one.
    pub(super) fn settle<T>(&self, result: Result<T, EngineError>) -> Result<T, EngineError> {
        let mut job_error = None;
        // Asking is cheaper than running a job that is not there, and most operations
        // queue none.
        while self.runtime.is_job_pending() {
            match self.runtime.execute_pending_job() {
                Ok(true) => {}
                Ok(false) => break,
                Err(exception) => {
                    let error = exception.0.with(|ctx| {
                        caught_js_error(CaughtError::from_error(&ctx, rquickjs::Error::Exception))
                    });
                    job_error.get_or_insert(error);
                }
            }
        }
        self.harvest_tasks();
        let unhandled = self.rejections.take();
        if self.execution.budget_expired() {
            job_error.get_or_insert_with(|| EngineError::Execution {
                details: "execution budget exceeded".to_owned(),
            });
        }
        // A host function that panicked cannot unwind through QuickJS: the panic waited,
        // beyond reach of a script's `catch`, for the operation to end.
        host_fn::resume_panic();
        let value = result.map_err(|error| self.reported(error))?;
        match job_error.or(unhandled) {
            Some(error) => Err(self.reported(error)),
            None => Ok(value),
        }
    }

    /// An error as the host sees it: TypeScript locations, and an out of memory, however
    /// QuickJS or rquickjs described it, said as such with the limit to raise.
    fn reported(&self, error: EngineError) -> EngineError {
        match in_typescript(error, &self.module_store) {
            EngineError::Execution { details }
                if details.is_empty()
                    || details.starts_with(NULL_EXCEPTION)
                    || details.starts_with(OUT_OF_MEMORY) =>
            {
                let stats = self.memory_stats();
                if stats.malloc_limit_bytes != 0
                    && stats.malloc_size_bytes * 10 >= stats.malloc_limit_bytes * 9
                {
                    return EngineError::Execution {
                        details: format!(
                            "out of memory: {} of {} bytes allocated, the limit \
                             `EngineOptions::memory_limit_bytes`; raise it or load fewer scripts",
                            stats.malloc_size_bytes, stats.malloc_limit_bytes
                        ),
                    };
                }
                EngineError::Execution { details }
            }
            error => error,
        }
    }

    /// Reports an operation that failed after an [`InterruptHandle`] request as
    /// [`EngineError::Interrupted`] rather than as the QuickJS interrupt exception.
    pub(super) fn attribute_interrupt<T>(
        &self,
        result: Result<T, EngineError>,
    ) -> Result<T, EngineError> {
        match result {
            Err(_) if self.execution.interrupt_requested() => Err(EngineError::Interrupted),
            result => result,
        }
    }
}
