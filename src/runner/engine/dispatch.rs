//! Calling exports and delivering events and requests, and the settling that ends
//! every operation.

use rquickjs::{CatchResultExt, CaughtError, Ctx, Value as JsValue};

use crate::contract::{JsArgs, JsDecode, JsEncode, ScriptExport};
use crate::error::EngineError;
use crate::types::Emission;

use super::super::errors::{caught_js_error, in_typescript};
use super::super::host_fn;
use super::Engine;
use super::delivery::Audience;
use super::invoke::invoke;

/// How a thrown `null` is described: QuickJS throws it when it runs out of memory while
/// already handling an out of memory. Built without its assertions, the same failure
/// surfaces as an exception with no message at all.
pub(super) const NULL_EXCEPTION: &str = "non-error exception: Null";

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
        let _charge = self.charge_to(script);
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
        let result = self.explain_quota(
            script_id,
            script,
            || format!("function `{function}`"),
            result,
        );
        self.attribute_interrupt(self.settle(result))
    }

    /// Calls the export declared by `E`, like [`Engine::call`], with its name, argument
    /// list and result type taken from the [`ScriptExport`] so that none can be misspelled
    /// or mistyped at the call.
    pub fn call_typed<E: ScriptExport>(
        &self,
        script_id: &str,
        args: E::Args,
    ) -> Result<E::Output, EngineError> {
        self.call(script_id, E::NAME, args)
    }

    /// Delivers one event to every handler registered for it, script by script in load
    /// order; returns the number of scripts that had at least one handler. A throwing
    /// handler does not stop the others: every handler runs, then the first error is
    /// returned. An interrupt or an exhausted execution budget does stop them: no
    /// handler is entered after it. The payload is encoded once per script with
    /// handlers and shared by all of that script's handlers. Scripts that never
    /// registered a handler for `event` cost a set lookup. To know which script
    /// failed, use [`emit_report`](Self::emit_report), or [`emit_to`](Self::emit_to)
    /// for one script.
    pub fn emit<P: JsEncode + ?Sized>(
        &self,
        event: &str,
        payload: &P,
    ) -> Result<usize, EngineError> {
        let audience = self.everyone_listening_to(event);
        let emission = self.dispatch(&audience, event, payload, false, |_, _, _| Ok(()));
        self.first_failure(emission)
    }

    /// Delivers one event like [`Engine::emit`], and reports what it did to each script:
    /// how many it reached, the first error of every script whose handlers failed, and
    /// the errors no script owns. It never fails as a whole, so a host that disables
    /// the scripts that fail (or counts their failures) reads them all from one
    /// emission.
    pub fn emit_report<P: JsEncode + ?Sized>(&self, event: &str, payload: &P) -> Emission<'_> {
        let audience = self.everyone_listening_to(event);
        let mut emission = self.dispatch(&audience, event, payload, false, |_, _, _| Ok(()));
        if emission.delivered > 0
            && self.execution.interrupt_requested()
            && emission.unattributed.is_none()
        {
            emission.unattributed = Some(EngineError::Interrupted);
        }
        emission
    }

    /// Delivers one event to the handlers of one script only, like [`Engine::emit`]: every
    /// handler of the script runs even after one throws, and the first error is
    /// returned, so a failure is the script's by construction. Returns `1` if the script
    /// had a handler for `event`, `0` if it had none or is quarantined; an error if it is
    /// not loaded.
    pub fn emit_to<P: JsEncode + ?Sized>(
        &self,
        script_id: &str,
        event: &str,
        payload: &P,
    ) -> Result<usize, EngineError> {
        let audience = self.script_listening_to(script_id, event)?;
        let emission = self.dispatch(&audience, event, payload, false, |_, _, _| Ok(()));
        self.first_failure(emission)
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
        let audience = self.everyone_listening_to(event);
        self.request_audience(&audience, event, payload)
    }

    /// Delivers one event like [`Engine::request`], to the handlers of one script only,
    /// and returns their replies in registration order. The script not being loaded is
    /// an error; having no handler for `event`, or being quarantined, is an empty list.
    pub fn request_to<P: JsEncode + ?Sized, R: JsDecode>(
        &self,
        script_id: &str,
        event: &str,
        payload: &P,
    ) -> Result<Vec<R>, EngineError> {
        let audience = self.script_listening_to(script_id, event)?;
        let replies = self.request_audience(&audience, event, payload)?;
        Ok(replies.into_iter().map(|(_, reply)| reply).collect())
    }

    fn request_audience<P: JsEncode + ?Sized, R: JsDecode>(
        &self,
        audience: &Audience,
        event: &str,
        payload: &P,
    ) -> Result<Vec<(&str, R)>, EngineError> {
        let mut replies = Vec::new();
        let emission = self.dispatch(
            audience,
            event,
            payload,
            true,
            |script_id, ctx, returned| {
                let value = self.await_returned(ctx, returned, || {
                    format!("a `{event}` handler of script `{script_id}`")
                })?;
                let reply = R::decode_js(ctx, value)
                    .catch(ctx)
                    .map_err(caught_js_error)?;
                replies.push((script_id, reply));
                Ok(())
            },
        );
        self.first_failure(emission)?;
        Ok(replies)
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
    pub(super) fn reported(&self, error: EngineError) -> EngineError {
        match in_typescript(error, &self.module_store) {
            EngineError::Execution { details } if self.is_out_of_memory(&details) => {
                let stats = self.memory_stats();
                EngineError::Execution {
                    details: format!(
                        "out of memory: {} of {} bytes allocated, the limit \
                         `EngineOptions::memory_limit_bytes`; raise it or load fewer scripts",
                        stats.malloc_size_bytes, stats.malloc_limit_bytes
                    ),
                }
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
