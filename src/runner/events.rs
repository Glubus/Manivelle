//! The engine's copy of each context's event handler lists, and their delivery.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::rc::{Rc, Weak};

use rquickjs::{Array, Ctx, Exception, Function, Value as JsValue};
use rustc_hash::FxBuildHasher;

use super::errors::js_error;
use super::execution::ExecutionControl;
use super::retained::{Retained, release_value, take_value};
use crate::error::EngineError;
use crate::packages::PermissionSet;

/// Handler lists of one context, by event name, as the prelude hands them over: only
/// events with at least one handler are listed. Each list is kept as a snapshot of
/// the functions themselves, so delivering an event creates no property key, reads
/// no global and no array, and a context without handlers for the event is not
/// entered at all. A change replaces the snapshot: a delivery in progress keeps
/// running the one it took.
///
/// The script owns the only strong reference. The prelude's hook holds a weak one:
/// the snapshots keep the context's functions alive, so a strong reference from
/// inside the context would be a cycle the garbage collector cannot see.
pub(super) struct ListenedEvents {
    list: Rc<RefCell<Vec<Listened>>>,
    /// The engine's [`EventIndex`] flag, raised when an event gains its first handler
    /// or loses its last.
    stale: Rc<Cell<bool>>,
}

/// The handlers of one event, in registration order.
pub(super) type Handlers = Rc<[Retained]>;

struct Listened {
    event: Box<str>,
    handlers: Handlers,
}

impl ListenedEvents {
    pub(super) fn new(stale: Rc<Cell<bool>>) -> Self {
        Self {
            list: Rc::default(),
            stale,
        }
    }

    /// Calls `visit` with the name of each event the script has a handler for.
    fn for_each_event(&self, mut visit: impl FnMut(&str)) {
        for listened in self.list.borrow().iter() {
            visit(&listened.event);
        }
    }

    /// Whether the script has a handler for `event`, without taking its handlers.
    pub(super) fn listens(&self, event: &str) -> bool {
        self.list
            .borrow()
            .iter()
            .any(|listened| &*listened.event == event)
    }

    /// The current handlers of `event`, `None` when the context has none. The borrow
    /// ends before any JavaScript runs, so handlers may call `ctx.on` and `ctx.off`.
    #[inline]
    pub(super) fn handlers(&self, event: &str) -> Option<Handlers> {
        self.list
            .borrow()
            .iter()
            .find(|listened| &*listened.event == event)
            .map(|listened| Rc::clone(&listened.handlers))
    }

    /// The hook the prelude hands handler lists to. A script of a package whose
    /// permissions do not list `event` cannot add a handler for it: the call throws.
    pub(super) fn hook<'js>(
        &self,
        ctx: &Ctx<'js>,
        script_id: &str,
        allowed: Option<Rc<PermissionSet>>,
    ) -> Result<Function<'js>, EngineError> {
        let events = Rc::downgrade(&self.list);
        let stale = Rc::clone(&self.stale);
        let script_id = Box::<str>::from(script_id);
        Function::new(
            ctx.clone(),
            move |ctx: Ctx<'_>, event: String, handlers: Option<Array<'_>>| {
                if handlers.is_some() && allowed.as_ref().is_some_and(|set| !set.allows(&event)) {
                    let denied = EngineError::PermissionDenied {
                        package: script_id.to_string(),
                        permission: event,
                    };
                    return Err(Exception::throw_message(&ctx, &denied.to_string()));
                }
                let handlers = handlers.map(|list| snapshot(&list)).transpose()?;
                record(&events, &stale, event, handlers);
                rquickjs::Result::Ok(())
            },
        )
        .map_err(js_error)
    }
}

/// The functions of a handler list the prelude built; it only holds functions.
fn snapshot(list: &Array<'_>) -> rquickjs::Result<Handlers> {
    list.iter::<Function<'_>>()
        .map(|handler| handler.map(|handler| Retained::new(handler.as_value())))
        .collect()
}

/// Stores `handlers` as the list of `event`, or forgets the event when `None`.
fn record(
    events: &Weak<RefCell<Vec<Listened>>>,
    stale: &Cell<bool>,
    event: String,
    handlers: Option<Handlers>,
) {
    let Some(events) = events.upgrade() else {
        return;
    };
    let mut events = events.borrow_mut();
    let position = events.iter().position(|listened| *listened.event == *event);
    match (position, handlers) {
        (Some(position), Some(handlers)) => events[position].handlers = handlers,
        (None, Some(handlers)) => {
            events.push(Listened {
                event: event.into_boxed_str(),
                handlers,
            });
            stale.set(true);
        }
        (Some(position), None) => {
            events.swap_remove(position);
            stale.set(true);
        }
        (None, None) => {}
    }
}

/// Which scripts have a handler for each event, as positions in load order, so an
/// emission visits the scripts that listen and no others.
///
/// Rebuilt on the first use after the set of listening scripts may have changed (a
/// script loaded, replaced or unloaded, an event gaining its first handler or losing
/// its last), which costs one pass over the scripts, what each emission paid before.
/// A handler registered while an event is delivered is served from the next emission.
#[derive(Default)]
pub(super) struct EventIndex {
    stale: Rc<Cell<bool>>,
    listeners: RefCell<HashMap<Box<str>, Rc<[usize]>, FxBuildHasher>>,
}

impl EventIndex {
    /// The flag a script's [`ListenedEvents`] raises.
    pub(super) fn stale_flag(&self) -> Rc<Cell<bool>> {
        Rc::clone(&self.stale)
    }

    pub(super) fn mark_stale(&self) {
        self.stale.set(true);
    }

    /// The positions, in load order, of the scripts listening to `event`; `None` when
    /// no script does. `scripts` yields every script's events in load order, `None` for a
    /// script that must receive none (a quarantined one), and is only read when the
    /// index is stale.
    pub(super) fn listeners<'a>(
        &self,
        event: &str,
        scripts: impl Iterator<Item = Option<&'a ListenedEvents>>,
    ) -> Option<Rc<[usize]>> {
        if self.stale.get() {
            self.rebuild(scripts);
        }
        self.listeners.borrow().get(event).cloned()
    }

    fn rebuild<'a>(&self, scripts: impl Iterator<Item = Option<&'a ListenedEvents>>) {
        let mut by_event: HashMap<Box<str>, Vec<usize>, FxBuildHasher> = HashMap::default();
        for (position, events) in scripts.enumerate() {
            let Some(events) = events else {
                continue;
            };
            events.for_each_event(|name| match by_event.get_mut(name) {
                Some(positions) => positions.push(position),
                None => {
                    by_event.insert(Box::from(name), vec![position]);
                }
            });
        }
        *self.listeners.borrow_mut() = by_event
            .into_iter()
            .map(|(event, positions)| (event, Rc::from(positions)))
            .collect();
        self.stale.set(false);
    }
}

/// Runs every handler with `payload`, the encoded event, even after one throws; returns
/// the first error of a handler or of `on_return`. With `keep_results`, each returned
/// value goes to `on_return`; without, it is dropped unread, which skips building a
/// value around it. Once the operation is interrupted or out of budget, no further
/// handler is entered: the operation reports that error.
///
/// The handler and the payload are only borrowed for each call: delivering to a handler
/// takes no reference, where `Function::call` would take and release several.
pub(super) fn deliver<'js>(
    ctx: &Ctx<'js>,
    handlers: &[Retained],
    payload: &JsValue<'js>,
    keep_results: bool,
    execution: &ExecutionControl,
    mut on_return: impl FnMut(JsValue<'js>) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    let mut first_error = None;
    for handler in handlers {
        let outcome = call_handler(ctx, handler, payload, keep_results, &mut on_return);
        if let Err(error) = outcome {
            first_error.get_or_insert(error);
        }
        if execution.should_stop() {
            break;
        }
    }
    first_error.map_or(Ok(()), Err)
}

/// Calls one handler with the payload, and hands what it returned to `on_return`, or drops
/// it unread.
fn call_handler<'js>(
    ctx: &Ctx<'js>,
    handler: &Retained,
    payload: &JsValue<'js>,
    keep_results: bool,
    on_return: &mut impl FnMut(JsValue<'js>) -> Result<(), EngineError>,
) -> Result<(), EngineError> {
    let returned = handler.call(ctx, &mut [payload.as_raw()])?;
    if keep_results {
        // SAFETY: `returned` is the owned value `call` gave back, which nothing else holds.
        on_return(unsafe { take_value(ctx, returned) })
    } else {
        // SAFETY: as above, and it is not used again.
        unsafe { release_value(ctx, returned) };
        Ok(())
    }
}
