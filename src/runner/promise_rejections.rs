//! Promise rejections that no handler caught, reported when an operation ends.

use std::cell::RefCell;
use std::rc::Rc;

use rquickjs::{Ctx, Persistent, Runtime, Value};

use crate::error::EngineError;

use super::errors::js_value_details;

/// Records rejected Promises without a handler, and forgets them once one is attached.
///
/// QuickJS reports a rejection as unhandled when it happens, then as handled if a
/// handler is attached later, so only the rejections still pending when an operation
/// ends are errors.
pub(crate) struct UnhandledRejections(Rc<RefCell<Vec<Rejection>>>);

struct Rejection {
    promise: Persistent<Value<'static>>,
    details: String,
}

impl UnhandledRejections {
    /// Installs the tracker on `runtime`.
    pub(crate) fn install(runtime: &Runtime) -> Self {
        let pending = Rc::new(RefCell::new(Vec::<Rejection>::new()));
        let tracked = Rc::clone(&pending);
        runtime.set_host_promise_rejection_tracker(Some(Box::new(
            move |ctx: Ctx<'_>, promise: Value<'_>, reason: Value<'_>, is_handled: bool| {
                let mut tracked = tracked.borrow_mut();
                if is_handled {
                    tracked.retain(|rejection| !rejection.is(&ctx, &promise));
                } else {
                    tracked.push(Rejection {
                        promise: Persistent::save(&ctx, promise),
                        details: js_value_details(&reason),
                    });
                }
            },
        )));
        Self(pending)
    }

    /// Stops tracking `promise`, whose rejection the caller reports itself.
    pub(crate) fn forget<'js>(&self, ctx: &Ctx<'js>, promise: &Value<'js>) {
        self.0
            .borrow_mut()
            .retain(|rejection| !rejection.is(ctx, promise));
    }

    /// Clears every rejection still unhandled and reports them all, one line each, or
    /// returns `None` when there are none.
    pub(crate) fn take(&self) -> Option<EngineError> {
        let pending = std::mem::take(&mut *self.0.borrow_mut());
        if pending.is_empty() {
            return None;
        }
        let details = pending
            .iter()
            .map(|rejection| format!("unhandled promise rejection: {}", rejection.details))
            .collect::<Vec<_>>()
            .join("\n");
        Some(EngineError::Execution { details })
    }
}

/// Releases the saved Promises while their runtime is still alive.
impl Drop for UnhandledRejections {
    fn drop(&mut self) {
        self.0.borrow_mut().clear();
    }
}

impl Rejection {
    fn is<'js>(&self, ctx: &Ctx<'js>, promise: &Value<'js>) -> bool {
        self.promise
            .clone()
            .restore(ctx)
            .is_ok_and(|saved| saved == *promise)
    }
}

#[cfg(test)]
mod tests {
    use rquickjs::{Context, Runtime};

    use super::UnhandledRejections;
    use crate::error::EngineError;

    #[test]
    fn take_reports_every_unhandled_rejection() {
        let runtime = Runtime::new().expect("runtime");
        let tracker = UnhandledRejections::install(&runtime);
        let context = Context::full(&runtime).expect("context");
        context
            .with(|ctx| ctx.eval::<(), _>("Promise.reject('first'); Promise.reject('second');"))
            .expect("reject");

        let Some(EngineError::Execution { details }) = tracker.take() else {
            panic!("two rejections are reported");
        };
        assert!(
            details.contains("first") && details.contains("second"),
            "{details}"
        );
        assert!(tracker.take().is_none(), "take clears the rejections");
    }
}
