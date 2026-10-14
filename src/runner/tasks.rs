//! Results of JavaScript operations that stay suspended between host calls.

use std::cell::RefCell;
use std::rc::{Rc, Weak};

use rquickjs::function::This;
use rquickjs::prelude::Func;
use rquickjs::{CatchResultExt, Ctx, Persistent, Promise, Value as JsValue};

use crate::contract::JsDecode;
use crate::error::EngineError;

use super::errors::{caught_js_error, in_typescript, js_error};
use super::module_loader::ModuleStore;
use super::promise_rejections::UnhandledRejections;

/// A script result that can complete after a later host-driven operation. Polling or
/// taking the result never executes JavaScript; the engine drives it through `pump`,
/// `advance_timers`, and other script operations.
pub struct PendingCall<R> {
    result: Rc<RefCell<Option<Result<R, EngineError>>>>,
}

impl<R> Clone for PendingCall<R> {
    fn clone(&self) -> Self {
        Self {
            result: Rc::clone(&self.result),
        }
    }
}

impl<R> PendingCall<R> {
    pub(super) fn new() -> Self {
        Self {
            result: Rc::new(RefCell::new(None)),
        }
    }

    pub(super) fn complete(&self, value: Result<R, EngineError>) {
        let mut result = self.result.borrow_mut();
        if result.is_none() {
            *result = Some(value);
        }
    }

    /// Whether the script result has completed. A result already taken is no longer
    /// available and reports false here.
    pub fn is_finished(&self) -> bool {
        self.result.borrow().is_some()
    }

    /// Takes the finished result once, or returns `None` while it is still pending or
    /// after another clone of the handle has taken it.
    pub fn take(&self) -> Option<Result<R, EngineError>> {
        self.result.borrow_mut().take()
    }
}

/// A weak generation-local reference to a request that may already have received
/// this script's reply but still awaits another script.
pub(super) struct RequestGuard(Weak<dyn RequestLifetime>);

trait RequestLifetime {
    fn is_pending(&self) -> bool;
    fn cancel(&self);
}

impl RequestGuard {
    pub(super) fn new<R: JsDecode + 'static>(state: &Rc<RefCell<RequestResults<R>>>) -> Self {
        let lifetime = Rc::clone(state) as Rc<dyn RequestLifetime>;
        Self(Rc::downgrade(&lifetime))
    }

    pub(super) fn is_pending(&self) -> bool {
        self.0.upgrade().is_some_and(|request| request.is_pending())
    }

    pub(super) fn cancel(&self) {
        if let Some(request) = self.0.upgrade() {
            request.cancel();
        }
    }
}

impl<R: JsDecode + 'static> RequestLifetime for RefCell<RequestResults<R>> {
    fn is_pending(&self) -> bool {
        !self.borrow().finished
    }

    fn cancel(&self) {
        let mut request = self.borrow_mut();
        if !request.finished {
            request.finished = true;
            request.handle.complete(Err(EngineError::Cancelled));
        }
    }
}

/// Aggregates handler replies in registration order, even when Promise resolution
/// happens in another order. All handlers run before the result is published.
pub(super) struct RequestResults<R> {
    handle: PendingCall<Vec<(String, R)>>,
    replies: Vec<Option<Result<(String, R), EngineError>>>,
    active: bool,
    finished: bool,
}

impl<R: JsDecode + 'static> RequestResults<R> {
    pub(super) fn new(handle: PendingCall<Vec<(String, R)>>) -> Rc<RefCell<Self>> {
        Rc::new(RefCell::new(Self {
            handle,
            replies: Vec::new(),
            active: false,
            finished: false,
        }))
    }

    pub(super) fn observe<'js>(
        state: &Rc<RefCell<Self>>,
        ctx: &Ctx<'js>,
        script_id: &str,
        value: JsValue<'js>,
    ) -> Result<Option<ScriptTask>, EngineError> {
        let slot = {
            let mut state = state.borrow_mut();
            let slot = state.replies.len();
            state.replies.push(None);
            slot
        };
        let id = script_id.to_owned();
        let completed = Rc::clone(state);
        let cancelled = state.borrow().handle.clone();
        ScriptTask::watch(
            ctx,
            value,
            Box::new(move |ctx, outcome| {
                let reply = outcome
                    .and_then(|value| R::decode_js(ctx, value).catch(ctx).map_err(caught_js_error));
                completed
                    .borrow_mut()
                    .record(slot, reply.map(|reply| (id, reply)));
            }),
            Box::new(move || cancelled.complete(Err(EngineError::Cancelled))),
        )
    }

    pub(super) fn fail(&mut self, error: EngineError) {
        self.replies.push(Some(Err(error)));
        self.complete_if_ready();
    }

    pub(super) fn activate(&mut self) {
        self.active = true;
        self.complete_if_ready();
    }

    fn record(&mut self, slot: usize, result: Result<(String, R), EngineError>) {
        self.replies[slot] = Some(result);
        self.complete_if_ready();
    }

    fn complete_if_ready(&mut self) {
        if self.finished || !self.active || self.replies.iter().any(Option::is_none) {
            return;
        }
        let mut first_error = None;
        let mut replies = Vec::with_capacity(self.replies.len());
        for slot in self.replies.drain(..) {
            match slot.expect("all request handlers have completed") {
                Ok(reply) => replies.push(reply),
                Err(error) => {
                    first_error.get_or_insert(error);
                }
            }
        }
        self.active = false;
        self.finished = true;
        self.handle.complete(first_error.map_or(Ok(replies), Err));
    }
}

type TaskFinish = Box<dyn for<'js> FnOnce(&Ctx<'js>, Result<JsValue<'js>, EngineError>)>;

pub(super) struct ScriptTask {
    promise: Persistent<Promise<'static>>,
    finish: Option<TaskFinish>,
    cancel: Option<Box<dyn FnOnce()>>,
}

impl ScriptTask {
    pub(super) fn observe<'js, R: JsDecode + 'static>(
        ctx: &Ctx<'js>,
        value: JsValue<'js>,
        handle: &PendingCall<R>,
    ) -> Result<Option<Self>, EngineError> {
        let result = handle.clone();
        let cancelled = handle.clone();
        Self::watch(
            ctx,
            value,
            Box::new(move |ctx, value| {
                result.complete(value.and_then(|value| {
                    R::decode_js(ctx, value).catch(ctx).map_err(caught_js_error)
                }));
            }),
            Box::new(move || cancelled.complete(Err(EngineError::Cancelled))),
        )
    }

    pub(super) fn watch<'js>(
        ctx: &Ctx<'js>,
        value: JsValue<'js>,
        finish: TaskFinish,
        cancel: Box<dyn FnOnce()>,
    ) -> Result<Option<Self>, EngineError> {
        let Some(promise) = value.as_promise() else {
            finish(ctx, Ok(value));
            return Ok(None);
        };
        // Mark the original rejection handled before jobs run. Reading the original
        // promise still reports its rejection to the pending Rust handle.
        promise
            .catch()
            .map_err(js_error)?
            .call::<_, JsValue<'js>>((
                This(promise.clone()),
                Func::from(|_reason: JsValue<'js>| {}),
            ))
            .map_err(js_error)?;
        Ok(Some(Self {
            promise: Persistent::save(ctx, promise.clone()),
            finish: Some(finish),
            cancel: Some(cancel),
        }))
    }

    /// Returns true when this task no longer needs to retain its JavaScript promise.
    pub(super) fn poll(
        &mut self,
        ctx: &Ctx<'_>,
        rejections: &UnhandledRejections,
        modules: &ModuleStore,
    ) -> bool {
        let promise = match self.promise.clone().restore(ctx) {
            Ok(promise) => promise,
            Err(error) => {
                if let Some(finish) = self.finish.take() {
                    finish(ctx, Err(js_error(error)));
                }
                self.cancel.take();
                return true;
            }
        };
        let Some(result) = promise.result::<JsValue<'_>>() else {
            return false;
        };
        rejections.forget(ctx, promise.as_value());
        if let Some(finish) = self.finish.take() {
            finish(
                ctx,
                result
                    .catch(ctx)
                    .map_err(|error| in_typescript(caught_js_error(error), modules)),
            );
        }
        self.cancel.take();
        true
    }
}

impl Drop for ScriptTask {
    fn drop(&mut self) {
        if let Some(cancel) = self.cancel.take() {
            cancel();
        }
    }
}
