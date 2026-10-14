//! Delivering an event to the scripts that listen to it, in load order.
//!
//! The scripts that follow one another in load order and share a context (a group's) are
//! served in one visit to it: the context is entered once and the payload encoded once,
//! instead of once per script.

use std::rc::Rc;

use rquickjs::{Ctx, Value as JsValue};

use super::super::errors::js_error;
use super::super::events::{Handlers, deliver};
use super::{Engine, EngineScript};
use crate::contract::JsEncode;
use crate::error::EngineError;

impl Engine {
    /// Runs every handler of `event` in load order, handing each returned value to
    /// `on_return`; see [`Engine::emit`]. Returns how many scripts it delivered to.
    pub(super) fn dispatch<'a, P: JsEncode + ?Sized>(
        &'a self,
        event: &str,
        payload: &P,
        keep_results: bool,
        on_return: impl for<'js> FnMut(&'a str, &Ctx<'js>, JsValue<'js>) -> Result<(), EngineError>,
    ) -> Result<usize, EngineError> {
        let Some(listeners) = self.listeners_of(event) else {
            return Ok(0);
        };
        let _budget = self.budget();
        let mut delivery = Delivery {
            engine: self,
            event,
            payload,
            keep_results,
            on_return,
            delivered: 0,
            first_error: None,
            stopped: false,
        };
        delivery.visit(&listeners);
        delivery.finish()
    }

    /// The load-order positions of the scripts listening to `event`, quarantined ones
    /// left out; `None` when no script does.
    fn listeners_of(&self, event: &str) -> Option<Rc<[usize]>> {
        self.event_index.listeners(
            event,
            self.scripts
                .values()
                .map(|script| (!script.quarantined.get()).then_some(&script.signals.events)),
        )
    }

    /// The end, exclusive, of the run of scripts that starts at `start`, whose first script
    /// is `first`, and shares a context with it: `position(i)` is the script at index `i`
    /// of what is being visited, `len` its length.
    pub(super) fn context_run_end(
        &self,
        first: &EngineScript,
        start: usize,
        len: usize,
        position: impl Fn(usize) -> usize,
    ) -> usize {
        let mut end = start + 1;
        while end < len
            && self
                .scripts
                .get_index(position(end))
                .is_some_and(|(_, next)| first.shares_context(next))
        {
            end += 1;
        }
        end
    }
}

/// An emission in progress: what is delivered, and what happened so far.
struct Delivery<'a, 'e, P: ?Sized, F> {
    engine: &'a Engine,
    event: &'e str,
    payload: &'e P,
    keep_results: bool,
    on_return: F,
    /// The scripts a handler list was taken from.
    delivered: usize,
    /// The first error of a script, as the operation reports it.
    first_error: Option<EngineError>,
    /// Set once the host interrupts the operation: no handler is entered after that.
    /// A script that runs out of its own budget does not stop the others.
    stopped: bool,
}

impl<'a, 'e, P, F> Delivery<'a, 'e, P, F>
where
    P: JsEncode + ?Sized,
    F: for<'js> FnMut(&'a str, &Ctx<'js>, JsValue<'js>) -> Result<(), EngineError>,
{
    /// Delivers to the scripts at `listeners`, one context visit per run of scripts that
    /// share a context.
    fn visit(&mut self, listeners: &[usize]) {
        let engine = self.engine;
        let mut start = 0;
        while start < listeners.len() && !self.stopped {
            let Some((_, first)) = engine.scripts.get_index(listeners[start]) else {
                return;
            };
            let end = engine.context_run_end(first, start, listeners.len(), |i| listeners[i]);
            self.visit_run(first, &listeners[start..end]);
            start = end;
        }
    }

    /// Delivers to the scripts of one run, entering their shared context once.
    fn visit_run(&mut self, first: &EngineScript, run: &[usize]) {
        first
            .context
            .with(|ctx| match self.payload.encode_js(&ctx) {
                Ok(payload) => {
                    for &position in run {
                        self.deliver_to(position, &ctx, &payload);
                        if self.stopped {
                            break;
                        }
                    }
                }
                Err(error) => self.encoding_failed(run, js_error(error)),
            });
    }

    /// Delivers the encoded `payload` to the script at `position`, under a slice of its
    /// own. An earlier handler may have removed its handlers since the index was built.
    fn deliver_to<'js>(&mut self, position: usize, ctx: &Ctx<'js>, payload: &JsValue<'js>) {
        let Some((id, script)) = self.engine.scripts.get_index(position) else {
            return;
        };
        let Some(handlers) = script.signals.events.handlers(self.event) else {
            return;
        };
        self.delivered += 1;
        let outcome = self.run_handlers(ctx, id, script, &handlers, payload);
        if let Err(error) = outcome {
            self.first_error.get_or_insert(error);
        }
        self.stopped = self.engine.execution.interrupt_requested();
    }

    fn run_handlers<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        id: &'a str,
        script: &EngineScript,
        handlers: &Handlers,
        payload: &JsValue<'js>,
    ) -> Result<(), EngineError> {
        let event = self.event;
        let keep_results = self.keep_results;
        let engine = self.engine;
        let on_return = &mut self.on_return;
        engine.run_slice(
            ctx,
            id,
            script,
            || format!("a `{event}` handler"),
            || {
                deliver(
                    ctx,
                    handlers,
                    payload,
                    keep_results,
                    &engine.execution,
                    |returned| on_return(id, ctx, returned),
                )
            },
        )
    }

    /// The payload could not be encoded: nothing to hand over, so the scripts of the run
    /// that listen fail together.
    fn encoding_failed(&mut self, run: &[usize], error: EngineError) {
        self.first_error.get_or_insert(error);
        self.delivered += run
            .iter()
            .filter(|&&position| {
                self.engine
                    .scripts
                    .get_index(position)
                    .is_some_and(|(_, script)| script.signals.events.listens(self.event))
            })
            .count();
    }

    /// Settles what the handlers left, and reports the operation's result.
    fn finish(self) -> Result<usize, EngineError> {
        if self.delivered == 0 {
            // No JavaScript ran, so there is no Promise job to settle.
            return Ok(0);
        }
        // The work left to settle runs under a budget of its own.
        self.engine.begin_settle_slice();
        self.engine.attribute_interrupt(
            self.engine
                .settle(self.first_error.map_or(Ok(self.delivered), Err)),
        )
    }
}
