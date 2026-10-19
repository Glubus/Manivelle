//! JavaScript values the engine keeps and uses through the C API, and the atoms it keeps
//! for the names it looks up.

use std::cell::{Cell, RefCell};

use rquickjs::{Ctx, Value, qjs};
use rustc_hash::FxHashMap;

use super::errors::pending_exception;
use crate::error::EngineError;

/// A JavaScript value the engine keeps and uses without cloning anything.
///
/// A `Persistent` has to be cloned and restored to be used, which takes and gives back two
/// references (the value and its context) per use. This holds one reference to the value,
/// handed out as a raw `JSValue` that C API calls borrow. It does not keep its context
/// alive: the objects the engine keeps (functions, a module's namespace) hold their own
/// realm.
///
/// Like a `Persistent`, it must be dropped before the runtime it belongs to, on the thread
/// that owns the runtime; it is neither `Send` nor `Sync`, and a value leaked past the
/// runtime aborts it when it drops, as a leaked `Persistent` does.
pub(super) struct Retained {
    rt: *mut qjs::JSRuntime,
    value: qjs::JSValue,
}

impl Retained {
    pub(super) fn new(value: &Value<'_>) -> Self {
        let ctx = value.ctx().as_raw().as_ptr();
        // SAFETY: `ctx` is the live context `value` belongs to, and `as_raw` is a value of
        // it: the duplicate this takes is the reference `Drop` gives back.
        unsafe {
            Self {
                rt: qjs::JS_GetRuntime(ctx),
                value: qjs::JS_DupValue(ctx, value.as_raw()),
            }
        }
    }

    /// The value, borrowed: valid while `self` is.
    pub(super) fn raw(&self) -> qjs::JSValue {
        self.value
    }

    /// Calls the function with `this` undefined and `arguments`, which are only borrowed,
    /// and returns its result, an owned value of `ctx` the caller must hand to
    /// [`take_value`] or [`release_value`]. A function that threw is an error: the pending
    /// exception is taken from `ctx`.
    pub(super) fn call(
        &self,
        ctx: &Ctx<'_>,
        arguments: &mut [qjs::JSValue],
    ) -> Result<qjs::JSValue, EngineError> {
        let count = i32::try_from(arguments.len()).expect("a handful of arguments");
        // SAFETY: `self` holds a live function of this runtime, `arguments` holds values
        // of `ctx` that `JS_Call` only borrows, and `this` is `undefined`, which needs no
        // release.
        let returned = unsafe {
            qjs::JS_Call(
                ctx.as_raw().as_ptr(),
                self.value,
                qjs::JS_UNDEFINED,
                count,
                arguments.as_mut_ptr(),
            )
        };
        // SAFETY: `returned` is the value `JS_Call` just gave back.
        if unsafe { qjs::JS_IsException(returned) } {
            return Err(pending_exception(ctx));
        }
        Ok(returned)
    }

    /// Like [`Self::call`], but the result is released at once, unread.
    pub(super) fn call_discarding(
        &self,
        ctx: &Ctx<'_>,
        arguments: &mut [qjs::JSValue],
    ) -> Result<(), EngineError> {
        let returned = self.call(ctx, arguments)?;
        // SAFETY: `returned` is the owned value `call` gave back, which nothing else holds,
        // and it is not used again.
        unsafe { release_value(ctx, returned) };
        Ok(())
    }
}

/// Wraps an owned value [`Retained::call`] returned, which the wrapper releases.
///
/// # Safety
///
/// `value` must be an owned value of `ctx` that nothing else releases.
pub(super) unsafe fn take_value<'js>(ctx: &Ctx<'js>, value: qjs::JSValue) -> Value<'js> {
    // SAFETY: the caller's contract: an owned value of `ctx`, released by the wrapper.
    unsafe { Value::from_raw(ctx.clone(), value) }
}

/// Releases an owned value [`Retained::call`] returned and nobody wants; `undefined`, the
/// usual result, holds no reference and is skipped.
///
/// # Safety
///
/// `value` must be an owned value of `ctx` that nothing else releases, and not used again.
pub(super) unsafe fn release_value(ctx: &Ctx<'_>, value: qjs::JSValue) {
    // SAFETY: the caller's contract.
    unsafe {
        if !qjs::JS_IsUndefined(value) {
            qjs::JS_FreeValue(ctx.as_raw().as_ptr(), value);
        }
    }
}

impl Drop for Retained {
    fn drop(&mut self) {
        // SAFETY: `value` holds the reference `new` took, and the runtime is alive: see
        // the type's contract.
        unsafe { qjs::JS_FreeValueRT(self.rt, self.value) }
    }
}

/// The most names whose atoms are kept; a host that looks up more than this many distinct
/// names pays for an atom on each call past it.
const KEPT_ATOMS: usize = 1024;

/// The atoms of the export names the host calls, kept so that a call does not hash and
/// look the name up in the runtime's atom table every time. Only the name is kept: the
/// property is still read at each call, so a binding a module reassigns is seen.
///
/// Like [`Retained`], it must drop before the runtime.
pub(super) struct KeptAtoms {
    rt: Cell<*mut qjs::JSRuntime>,
    atoms: RefCell<FxHashMap<Box<str>, qjs::JSAtom>>,
}

impl Default for KeptAtoms {
    fn default() -> Self {
        Self {
            rt: Cell::new(std::ptr::null_mut()),
            atoms: RefCell::default(),
        }
    }
}

impl KeptAtoms {
    /// Runs `with` on the atom of `name`, or returns `None` when QuickJS has no memory
    /// for it, or cannot take a name that long. The atom is only valid during the call.
    pub(super) fn with<R>(
        &self,
        ctx: &Ctx<'_>,
        name: &str,
        with: impl FnOnce(qjs::JSAtom) -> R,
    ) -> Option<R> {
        if let Some(&atom) = self.atoms.borrow().get(name) {
            return Some(with(atom));
        }
        let raw = ctx.as_raw().as_ptr();
        // SAFETY: `raw` is a live context and `name` is valid for `name.len()` bytes.
        let atom = unsafe { qjs::JS_NewAtomLen(raw, name.as_ptr().cast(), name.len() as u64) };
        if atom == qjs::JS_ATOM_NULL {
            return None;
        }
        let mut atoms = self.atoms.borrow_mut();
        if atoms.len() < KEPT_ATOMS {
            // SAFETY: `raw` is live; its runtime outlives the cache by contract.
            self.rt.set(unsafe { qjs::JS_GetRuntime(raw) });
            atoms.insert(Box::from(name), atom);
            drop(atoms);
            return Some(with(atom));
        }
        drop(atoms);
        let result = with(atom);
        // SAFETY: the atom is a reference this call took and nothing kept.
        unsafe { qjs::JS_FreeAtom(raw, atom) };
        Some(result)
    }
}

impl Drop for KeptAtoms {
    fn drop(&mut self) {
        let rt = self.rt.get();
        for atom in self.atoms.get_mut().values() {
            // SAFETY: each atom is a reference the cache took, and the runtime is alive:
            // see the type's contract.
            unsafe { qjs::JS_FreeAtomRT(rt, *atom) };
        }
    }
}
