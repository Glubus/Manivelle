//! The host state that [`Engine::with_state`](crate::Engine::with_state) lends to host
//! functions for the duration of a scope.
//!
//! A host function is a `'static` closure that QuickJS calls, so it cannot capture a
//! `&mut` the host owns. The scope stores a lifetime-erased pointer to that `&mut` in a
//! slot every host function of the engine shares, and a function reaches it through
//! [`Caller::with_state`](crate::Caller::with_state).
//!
//! Two kinds of failure, kept apart:
//!
//! - what the host's code can cause (no scope, wrong type, a nested borrow, a nested
//!   scope) is an [`EngineError`] the caller can handle;
//! - what only a bug in this module can cause (the lent pointer changed under a borrow,
//!   the borrow flag disagrees with the borrows alive) is an `assert!`: the unsafe code
//!   relies on it, so the process stops rather than read through a wrong pointer. They
//!   are skipped while a panic unwinds, so they never replace the original panic.

use std::any::{Any, type_name};
use std::cell::Cell;
use std::fmt;
use std::marker::PhantomData;
use std::ptr::NonNull;
use std::rc::Rc;

use crate::error::EngineError;

/// The slot shared by an engine and the host functions of its scripts. It holds no state
/// outside a [`Scope`].
#[derive(Clone, Default)]
pub struct StateSlot(Rc<Slot>);

#[derive(Default)]
struct Slot {
    /// The state lent by the running scope. Set and cleared only by [`Scope`]; the
    /// pointee is the `&mut` the scope keeps borrowed.
    lent: Cell<Option<NonNull<dyn Any>>>,
    /// Whether a [`StateSlot::with`] holds the `&mut` right now.
    borrowed: Cell<bool>,
}

impl fmt::Debug for StateSlot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("StateSlot")
            .field("lent", &self.0.lent.get().is_some())
            .field("borrowed", &self.0.borrowed.get())
            .finish()
    }
}

/// The address of the state, without the vtable: what the assertions compare.
fn address(state: NonNull<dyn Any>) -> *mut () {
    state.cast::<()>().as_ptr()
}

impl StateSlot {
    /// Lends `state` until the returned [`Scope`] drops. The scope keeps `state` borrowed
    /// for its whole life, so nothing else can use it meanwhile.
    ///
    /// Fails with [`EngineError::StateScopeActive`] when a scope already lent a state.
    pub(crate) fn enter<'a>(&self, state: &'a mut dyn Any) -> Result<Scope<'a>, EngineError> {
        if self.0.lent.get().is_some() {
            return Err(EngineError::StateScopeActive);
        }
        assert!(
            !self.0.borrowed.get(),
            "a host function borrows a state while none is lent"
        );
        let lent = NonNull::from(state);
        self.0.lent.set(Some(lent));
        Ok(Scope {
            slot: self.clone(),
            address: address(lent),
            _state: PhantomData,
        })
    }

    /// Runs `f` with the lent state as a `T`.
    ///
    /// Errors: [`EngineError::NoState`] outside a scope, [`EngineError::StateBusy`] when
    /// an enclosing host function already holds the state, [`EngineError::StateType`]
    /// when the scope lent another type.
    pub(crate) fn with<T: Any, R>(&self, f: impl FnOnce(&mut T) -> R) -> Result<R, EngineError> {
        let lent = self.0.lent.get().ok_or(EngineError::NoState)?;
        if self.0.borrowed.replace(true) {
            return Err(EngineError::StateBusy);
        }
        // From here the flag is ours: `Borrow` clears it on every way out.
        let _borrow = Borrow {
            slot: &self.0,
            address: address(lent),
        };
        // SAFETY: `lent` was made from the `&mut dyn Any` a live `Scope` keeps borrowed,
        // and the scope clears the slot before that borrow ends, so the pointee is alive
        // and nothing else uses it. The engine is `!Send`, so no other thread reads it.
        // The `borrowed` flag, which this call just raised from `false`, rules out any
        // other `&mut` made from `lent` while this one lives. The pointer is to a
        // `dyn Any`, whose `'static` bound is exactly the one the slot erased.
        let state: &mut dyn Any = unsafe { &mut *lent.as_ptr() };
        let state = state.downcast_mut::<T>().ok_or(EngineError::StateType {
            expected: type_name::<T>(),
        })?;
        Ok(f(state))
    }
}

/// A lent state. Dropping it takes the state back.
#[must_use = "the state is lent only while the scope lives"]
pub(crate) struct Scope<'a> {
    slot: StateSlot,
    /// Where the state was when it was lent.
    address: *mut (),
    /// The scope owns the `&mut` for `'a`.
    _state: PhantomData<&'a mut dyn Any>,
}

impl Drop for Scope<'_> {
    fn drop(&mut self) {
        let slot = &self.slot.0;
        let lent = slot.lent.take();
        let borrowed = slot.borrowed.replace(false);
        if !std::thread::panicking() {
            assert!(
                lent.map(address) == Some(self.address),
                "the lent state changed during its scope"
            );
            assert!(
                !borrowed,
                "a host function still borrows the state when its scope ends"
            );
        }
    }
}

/// The borrow [`StateSlot::with`] holds, released on every way out of it.
struct Borrow<'a> {
    slot: &'a Slot,
    address: *mut (),
}

impl Drop for Borrow<'_> {
    fn drop(&mut self) {
        let lent = self.slot.lent.get();
        let borrowed = self.slot.borrowed.replace(false);
        if !std::thread::panicking() {
            assert!(
                lent.map(address) == Some(self.address),
                "the state was withdrawn while a host function borrowed it"
            );
            assert!(borrowed, "the borrow flag was lowered under a live borrow");
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn lends_and_takes_back() {
        let slot = StateSlot::default();
        let mut value = 5_u32;
        {
            let _scope = slot.enter(&mut value).unwrap();
            assert_eq!(slot.with(|n: &mut u32| std::mem::replace(n, 9)).unwrap(), 5);
        }
        assert_eq!(value, 9);
        assert!(matches!(
            slot.with(|_: &mut u32| ()),
            Err(EngineError::NoState)
        ));
    }

    #[test]
    fn refuses_a_second_scope() {
        let slot = StateSlot::default();
        let mut first = 1_u32;
        let mut second = 2_u32;
        let _scope = slot.enter(&mut first).unwrap();
        assert!(matches!(
            slot.enter(&mut second),
            Err(EngineError::StateScopeActive)
        ));
        // The refused attempt left the first state lent.
        assert_eq!(slot.with(|n: &mut u32| *n).unwrap(), 1);
    }

    #[test]
    fn refuses_a_nested_borrow_and_recovers() {
        let slot = StateSlot::default();
        let mut value = 1_u32;
        let _scope = slot.enter(&mut value).unwrap();
        let nested = slot
            .with(|_: &mut u32| slot.with(|_: &mut u32| ()))
            .unwrap();
        assert!(matches!(nested, Err(EngineError::StateBusy)));
        slot.with(|n: &mut u32| *n += 1).unwrap();
    }

    #[test]
    fn reports_the_wrong_type_and_recovers() {
        let slot = StateSlot::default();
        let mut value = 1_u32;
        let _scope = slot.enter(&mut value).unwrap();
        let error = slot.with(|_: &mut String| ()).unwrap_err();
        assert!(matches!(
            error,
            EngineError::StateType { expected } if expected.ends_with("String")
        ));
        slot.with(|n: &mut u32| *n += 1).unwrap();
    }

    #[test]
    fn a_panic_in_a_borrow_releases_it() {
        let slot = StateSlot::default();
        let mut value = 1_u32;
        let _scope = slot.enter(&mut value).unwrap();
        let panicked = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let _ = slot.with(|_: &mut u32| panic!("host function panics"));
        }));
        assert!(panicked.is_err());
        slot.with(|n: &mut u32| *n += 1).unwrap();
    }
}
