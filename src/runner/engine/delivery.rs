//! Delivering an event to the scripts that listen to it, in load order.
//!
//! The scripts that follow one another in load order and share a context (a group's) are
//! served in one visit to it: the context is entered once and the payload encoded once,
//! instead of once per script.

use std::rc::Rc;

use rquickjs::{Ctx, Value as JsValue};

use super::super::errors::js_error;
use super::super::events::{Handlers, deliver, deliver_discarding};
use super::{Engine, EngineScript, script_not_found};
use crate::contract::JsEncode;
use crate::error::EngineError;
use crate::types::{Emission, ScriptId};

/// The scripts an event is delivered to, as positions in load order.
pub(super) enum Audience {
    /// No script listens.
    Nobody,
    /// Every script that listens.
    Everyone(Rc<[usize]>),
    /// One script that listens.
    One(usize),
}

impl Audience {
    fn positions(&self) -> &[usize] {
        match self {
            Self::Nobody => &[],
            Self::Everyone(positions) => positions,
            Self::One(position) => std::slice::from_ref(position),
        }
    }
}

impl Engine {
    /// Runs every handler of `event` of the `audience` in load order, handing each returned
    /// value to `on_return`; see [`Engine::emit`]. Settles what the handlers left, and
    /// reports what happened to each script.
    pub(super) fn run_audience<'a, P: JsEncode + ?Sized>(
        &'a self,
        audience: &Audience,
        event: &str,
        payload: &P,
        keep_results: bool,
        on_return: impl for<'js> FnMut(&'a ScriptId, &Ctx<'js>, JsValue<'js>) -> Result<(), EngineError>,
    ) -> Emission {
        if matches!(audience, Audience::Nobody) {
            return Emission::default();
        }
        let _budget = self.budget();
        let mut delivery = Delivery {
            engine: self,
            event,
            payload,
            keep_results,
            on_return,
            delivered: 0,
            failures: Vec::new(),
            stopped: false,
        };
        delivery.visit(audience.positions());
        delivery.finish()
    }

    /// What [`Engine::emit`] returns for an emission: the first error, the interrupt
    /// taking precedence over any, else how many scripts it delivered to.
    pub(super) fn first_failure(&self, emission: Emission) -> Result<usize, EngineError> {
        let delivered = emission.delivered;
        let error = (emission.failures.into_iter().next())
            .map(|(_, error)| error)
            .or(emission.unattributed);
        self.attribute_interrupt(error.map_or(Ok(delivered), Err))
    }

    /// The scripts listening to `event`, quarantined ones left out.
    pub(super) fn everyone_listening_to(&self, event: &str) -> Audience {
        self.listeners_of(event)
            .map_or(Audience::Nobody, Audience::Everyone)
    }

    /// The script `script_id` if it listens to `event` and is not quarantined; an error when
    /// it is not loaded.
    pub(super) fn script_listening_to(
        &self,
        script_id: &str,
        event: &str,
    ) -> Result<Audience, EngineError> {
        let (position, _, script) =
            (self.scripts.get_full(script_id)).ok_or_else(|| script_not_found(script_id))?;
        let listening = !script.quarantined.get() && script.signals.events.listens(event);
        Ok(if listening {
            Audience::One(position)
        } else {
            Audience::Nobody
        })
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

    /// How many scripts share a context with `first`, the head of a run: `first` and the
    /// scripts at `following`, in order, for as long as they share it.
    pub(super) fn run_len(
        &self,
        first: &EngineScript,
        following: impl IntoIterator<Item = usize>,
    ) -> usize {
        1 + following
            .into_iter()
            .take_while(|&position| {
                (self.scripts.get_index(position))
                    .is_some_and(|(_, next)| first.shares_context(next))
            })
            .count()
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
    /// The error of each script whose handlers failed, in delivery order.
    failures: Vec<(ScriptId, EngineError)>,
    /// Set once the host interrupts the operation: no handler is entered after that.
    /// A script that runs out of its own budget does not stop the others.
    stopped: bool,
}

impl<'a, 'e, P, F> Delivery<'a, 'e, P, F>
where
    P: JsEncode + ?Sized,
    F: for<'js> FnMut(&'a ScriptId, &Ctx<'js>, JsValue<'js>) -> Result<(), EngineError>,
{
    /// Delivers to the scripts at `listeners`, one context visit per run of scripts that
    /// share a context.
    fn visit(&mut self, listeners: &[usize]) {
        let engine = self.engine;
        let mut start = 0;
        while start < listeners.len() && !self.stopped {
            let Some((_, first)) = engine.scripts.get_index(listeners[start]) else {
                // Unreachable while scripts change only through `&mut self`: a position
                // that is no longer a script is skipped, not the rest of the listeners.
                start += 1;
                continue;
            };
            let run = engine.run_len(first, listeners[start + 1..].iter().copied());
            self.visit_run(first, &listeners[start..start + run]);
            start += run;
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
                Err(error) => self.encoding_failed(run, js_error(error.into())),
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
            self.failures.push((id.clone(), error));
        }
        self.stopped = self.engine.execution.interrupt_requested();
    }

    fn run_handlers<'js>(
        &mut self,
        ctx: &Ctx<'js>,
        id: &'a ScriptId,
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
                if keep_results {
                    deliver(ctx, handlers, payload, &engine.execution, |returned| {
                        on_return(id, ctx, returned)
                    })
                } else {
                    deliver_discarding(ctx, handlers, payload, &engine.execution)
                }
            },
        )
    }

    /// The payload could not be encoded: nothing to hand over, so the scripts of the run
    /// that listen fail together, each with the same error.
    fn encoding_failed(&mut self, run: &[usize], error: EngineError) {
        let details = match error {
            EngineError::Execution { details } => details,
            other => other.to_string(),
        };
        for &position in run {
            let Some((id, script)) = self.engine.scripts.get_index(position) else {
                continue;
            };
            if script.signals.events.listens(self.event) {
                self.delivered += 1;
                let details = details.clone();
                self.failures
                    .push((id.clone(), EngineError::Execution { details }));
            }
        }
    }

    /// Settles what the handlers left, and reports what happened.
    fn finish(self) -> Emission {
        let Self {
            engine,
            delivered,
            failures,
            ..
        } = self;
        if delivered == 0 {
            // No JavaScript ran, so there is no Promise job to settle.
            return Emission::default();
        }
        // The work left to settle runs under a budget of its own.
        engine.begin_settle_slice();
        let failures = (failures.into_iter())
            .map(|(id, error)| (id, engine.reported(error)))
            .collect();
        let unattributed = engine.settle(Ok(())).err();
        Emission {
            delivered,
            failures,
            unattributed,
        }
    }
}
