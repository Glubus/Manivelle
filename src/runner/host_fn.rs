//! Host functions as callable objects of a QuickJS class of our own.
//!
//! A function made with `rquickjs::Function::new` is called through a generic layer that
//! clones the context and the arguments several times and dispatches through a table
//! (seven reference-count writes and three indirections for a one-argument host call). The
//! objects here keep a boxed Rust closure in their opaque slot and are called by one
//! `extern "C"` function that borrows the arguments straight from QuickJS.
//!
//! A script sees an ordinary function: `typeof` is `"function"`, and the engine gives it
//! its context's `Function.prototype` so that `call`, `apply` and `bind` work.

use std::any::Any;
use std::cell::RefCell;
use std::ffi::CString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::ptr::NonNull;

use rquickjs::{Ctx, Error, Result as JsResult, Value, qjs};

/// A host function body: what the script passed as its first argument (`null` when it
/// passed none), and what to return or throw.
pub(crate) type HostCall =
    Box<dyn for<'js> Fn(&Ctx<'js>, Value<'js>) -> JsResult<Value<'js>> + 'static>;

/// The callable class host functions are instances of, registered once per runtime.
#[derive(Clone, Copy)]
pub(crate) struct HostFnClass(qjs::JSClassID);

/// What a function object owns, behind its opaque pointer.
struct HostFn {
    call: HostCall,
}

thread_local! {
    /// The payload of a panic a host function raised. QuickJS code sits between the
    /// handler and the engine that called into it and cannot unwind, so the panic waits
    /// here, whatever a script's `try`/`catch` does, until [`resume_panic`] re-raises it.
    static PANIC: RefCell<Option<Box<dyn Any + Send>>> = const { RefCell::new(None) };
}

/// Re-raises the panic a host function left behind, if any. The engine calls it when an
/// operation ends, so the panic surfaces from the call that reached the handler.
pub(crate) fn resume_panic() {
    if let Some(payload) = PANIC.with(|slot| slot.borrow_mut().take()) {
        std::panic::resume_unwind(payload);
    }
}

/// Runs host code a script can reach through an rquickjs function, parking a panic of it
/// for [`resume_panic`] and returning an error, which the script sees as an exception it
/// can catch, instead of unwinding through QuickJS.
pub(crate) fn guarded<R>(host: impl FnOnce() -> R) -> JsResult<R> {
    catch_unwind(AssertUnwindSafe(host)).map_err(|payload| {
        park(payload);
        Error::new_from_js_message("host", "function", "a host function panicked")
    })
}

fn park(payload: Box<dyn Any + Send>) {
    PANIC.with(|slot| {
        slot.borrow_mut().get_or_insert(payload);
    });
}

impl HostFnClass {
    /// Registers the class on the runtime `ctx` belongs to.
    ///
    /// # Errors
    ///
    /// QuickJS ran out of memory.
    pub(crate) fn register(ctx: &Ctx<'_>) -> JsResult<Self> {
        let definition = qjs::JSClassDef {
            class_name: c"ManivelleHostFunction".as_ptr(),
            finalizer: Some(finalize),
            gc_mark: None,
            call: Some(call),
            exotic: std::ptr::null_mut(),
        };
        let mut id = 0;
        // SAFETY: `ctx` is live, so its runtime is. `definition` is read during the call
        // only (QuickJS copies the name), and `finalize` and `call` match the signatures
        // QuickJS expects.
        let registered = unsafe {
            let runtime = qjs::JS_GetRuntime(ctx.as_raw().as_ptr());
            qjs::JS_NewClassID(runtime, &mut id);
            qjs::JS_NewClass(runtime, id, &definition)
        };
        if registered < 0 {
            return Err(Error::Allocation);
        }
        Ok(Self(id))
    }

    /// A function object that runs `call` when a script calls it.
    pub(crate) fn function<'js>(self, ctx: &Ctx<'js>, call: HostCall) -> JsResult<Value<'js>> {
        let owned = Box::into_raw(Box::new(HostFn { call }));
        // SAFETY: `ctx` is live and the class is registered on its runtime. The object takes
        // ownership of `owned` on `JS_SetOpaque`, which `finalize` frees; if no object is
        // made, it is freed here.
        unsafe {
            let object = qjs::JS_NewObjectClass(ctx.as_raw().as_ptr(), self.0 as _);
            if qjs::JS_IsException(object) {
                drop(Box::from_raw(owned));
                return Err(Error::Exception);
            }
            qjs::JS_SetOpaque(object, owned.cast());
            Ok(Value::from_raw(ctx.clone(), object))
        }
    }
}

/// Frees what a collected function object owned.
unsafe extern "C" fn finalize(_runtime: *mut qjs::JSRuntime, object: qjs::JSValue) {
    // SAFETY: QuickJS calls this once per object of the class, with the object whose
    // opaque pointer `HostFnClass::function` set to a leaked `Box<HostFn>` (or never set).
    unsafe {
        let owned = qjs::JS_GetOpaque(object, qjs::JS_GetClassID(object)).cast::<HostFn>();
        if !owned.is_null() {
            drop(Box::from_raw(owned));
        }
    }
}

/// Runs when a script calls a host function: `argv` holds `argc` borrowed arguments.
unsafe extern "C" fn call(
    ctx: *mut qjs::JSContext,
    function: qjs::JSValue,
    _this: qjs::JSValue,
    argc: std::ffi::c_int,
    argv: *mut qjs::JSValue,
    _flags: std::ffi::c_int,
) -> qjs::JSValue {
    // SAFETY: QuickJS calls this for an object of the class, whose opaque pointer is a live
    // `HostFn` for as long as the object, which the running call keeps alive, is. A
    // missing opaque is reported as an internal error rather than dereferenced.
    let opaque = unsafe { qjs::JS_GetOpaque(function, qjs::JS_GetClassID(function)) };
    // SAFETY: `opaque` is null or the live `HostFn` of the class object, kept alive by
    // the running call; a null pointer makes `as_ref` return `None`.
    let Some(host) = (unsafe { opaque.cast::<HostFn>().as_ref() }) else {
        // SAFETY: `ctx` is the live context of the call; the message is a constant.
        return unsafe {
            qjs::JS_ThrowInternalError(
                ctx,
                c"%s".as_ptr(),
                c"a host function has no state".as_ptr(),
            )
        };
    };
    // SAFETY: QuickJS passes the live context of the call, never null, and `argv` points to
    // `argc` values of it.
    match catch_unwind(AssertUnwindSafe(|| unsafe { run(host, ctx, argc, argv) })) {
        Ok(value) => value,
        Err(payload) => {
            park(payload);
            // SAFETY: `ctx` is live; the message is a constant.
            unsafe {
                qjs::JS_ThrowInternalError(
                    ctx,
                    c"%s".as_ptr(),
                    c"a host function panicked".as_ptr(),
                )
            }
        }
    }
}

/// Calls `host` with the first argument, and turns its outcome into a value or an
/// exception, the way rquickjs does for its own functions.
///
/// # Safety
///
/// `ctx` is a live context and `argv` points to `argc` values of it.
unsafe fn run(
    host: &HostFn,
    ctx: *mut qjs::JSContext,
    argc: std::ffi::c_int,
    argv: *mut qjs::JSValue,
) -> qjs::JSValue {
    // SAFETY: the caller guarantees `ctx` is live and non-null.
    let ctx_ref = unsafe { Ctx::from_raw(NonNull::new_unchecked(ctx)) };
    let input = if argc > 0 {
        // SAFETY: `argv` holds at least one value; it is borrowed, so the wrapper owns a
        // reference of its own.
        unsafe { Value::from_raw(ctx_ref.clone(), qjs::JS_DupValue(ctx, *argv)) }
    } else {
        Value::new_null(ctx_ref.clone())
    };
    match (host.call)(&ctx_ref, input) {
        // SAFETY: `value` belongs to `ctx`; the duplicate is the reference handed to
        // QuickJS, and `value` releases its own as it drops.
        Ok(value) => unsafe { qjs::JS_DupValue(ctx, value.as_raw()) },
        Err(error) => throw(ctx, &error),
    }
}

/// Throws `error` in `ctx` as the exception a script catches, with the text rquickjs
/// would give it, and returns the value that tells QuickJS one is pending.
fn throw(ctx: *mut qjs::JSContext, error: &Error) -> qjs::JSValue {
    if matches!(error, Error::Exception) {
        // The conversion that failed already left its exception pending.
        return qjs::JS_EXCEPTION;
    }
    let message = error.to_string().replace('\0', "\u{fffd}");
    let message = CString::new(message).unwrap_or_default();
    // SAFETY: `ctx` is live; the format string consumes one `%s` argument, the message.
    unsafe {
        match error {
            Error::Allocation => qjs::JS_ThrowOutOfMemory(ctx),
            Error::AsSlice(_) | Error::Resolving { .. } | Error::Loading { .. } => {
                qjs::JS_ThrowReferenceError(ctx, c"%s".as_ptr(), message.as_ptr())
            }
            Error::InvalidString(_)
            | Error::Utf8(_)
            | Error::FromJs { .. }
            | Error::IntoJs { .. }
            | Error::TooManyArgs { .. }
            | Error::MissingArgs { .. } => {
                qjs::JS_ThrowTypeError(ctx, c"%s".as_ptr(), message.as_ptr())
            }
            _ => qjs::JS_ThrowInternalError(ctx, c"%s".as_ptr(), message.as_ptr()),
        }
    }
}
