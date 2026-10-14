//! The web APIs of [`ScriptBuiltins::web`](crate::ScriptBuiltins::web) that are not
//! QuickJS intrinsics: `structuredClone`, `TextEncoder` and `TextDecoder`.
//!
//! Their code is the module `assets/web_apis.js`, compiled once per engine and loaded
//! into a context only when a script first reads one of the globals, so creating a context
//! costs the three lazy declarations of the prelude's `installWebApis` and nothing more.
//! This file provides the function that loads the module and the Rust halves it calls
//! (UTF-8 conversion). None of them calls back into JavaScript, so a script cannot reach
//! another script's state through them, and none can block.

use std::cell::OnceCell;
use std::rc::Rc;

use rquickjs::{Ctx, Exception, Function, Module, Object, TypedArray, WriteOptions};

use super::errors::js_error;
use crate::error::EngineError;

/// The bytecode of the web module, compiled by the first context that reads one of the
/// globals and loaded by every later one.
pub(super) type WebBytecode = Rc<OnceCell<Box<[u8]>>>;

const WEB_MODULE: &str = "manivelle:web";
const WEB_SOURCE: &str = include_str!("../../assets/web_apis.js");

/// The function the prelude's `installWebApis` calls on the first read of a web global:
/// it loads the module into the calling context and returns its implementations, by
/// global name.
pub(super) fn loader<'js>(
    ctx: &Ctx<'js>,
    bytecode: &WebBytecode,
) -> Result<Function<'js>, EngineError> {
    let bytecode = Rc::clone(bytecode);
    let load = Function::new(
        ctx.clone(),
        move |ctx: Ctx<'js>| -> rquickjs::Result<Object<'js>> {
            let declared = match bytecode.get() {
                // SAFETY: QuickJS does not verify bytecode. These bytes were written by this
                // process, from the crate's own module, by the same QuickJS build.
                Some(bytes) => unsafe { Module::load(ctx.clone(), bytes)? },
                None => {
                    let declared = Module::declare(ctx.clone(), WEB_MODULE, WEB_SOURCE)?;
                    if let Ok(bytes) = declared.write(WriteOptions::default()) {
                        // Only this thread sets it, once: the first read.
                        let _ = bytecode.set(bytes.into_boxed_slice());
                    }
                    declared
                }
            };
            let (module, evaluated) = declared.eval()?;
            evaluated.finish::<()>()?;
            let create: Function<'js> = module.get("create")?;
            create.call((natives(&ctx)?,))
        },
    )
    .map_err(js_error)?;
    use_context_prototype(ctx, &load)?;
    Ok(load)
}

/// The object `create` of the module receives: `encode(text)` returns the UTF-8 bytes of a
/// string as a `Uint8Array`, `decode(bytes, fatal, ignoreBOM)` turns a `Uint8Array` into a
/// string.
fn natives<'js>(ctx: &Ctx<'js>) -> rquickjs::Result<Object<'js>> {
    let natives = Object::new(ctx.clone())?;
    // The bytes are copied into a buffer QuickJS allocates, so `memory_limit_bytes` counts
    // them: handing it the Rust `Vec` would keep them out of its accounting, and a loop
    // of `encode` calls would then grow the host's memory without bound.
    let encode = Function::new(ctx.clone(), |ctx: Ctx<'js>, text: String| {
        TypedArray::<u8>::new_copy(ctx, text.as_bytes())
    })?;
    let decode = Function::new(
        ctx.clone(),
        |ctx: Ctx<'js>,
         bytes: TypedArray<'js, u8>,
         fatal: bool,
         ignore_bom: bool|
         -> rquickjs::Result<String> {
            // SAFETY: the bytes are copied into the returned string before any
            // JavaScript can run again.
            let Some(bytes) = (unsafe { bytes.as_bytes() }) else {
                return Err(Exception::throw_type(&ctx, "the buffer is detached"));
            };
            let bytes = if ignore_bom {
                bytes
            } else {
                bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes)
            };
            if fatal {
                std::str::from_utf8(bytes).map(str::to_owned).map_err(|_| {
                    Exception::throw_type(&ctx, "The encoded data was not valid UTF-8")
                })
            } else {
                Ok(String::from_utf8_lossy(bytes).into_owned())
            }
        },
    )?;
    for function in [&encode, &decode] {
        use_context_prototype(ctx, function).map_err(|_| rquickjs::Error::Unknown)?;
    }
    natives.set("encode", encode)?;
    natives.set("decode", decode)?;
    Ok(natives)
}

/// rquickjs gives native functions the `Function.prototype` of the first context that made
/// one, for the whole runtime: gives `function` this context's, as `build_env` does.
fn use_context_prototype<'js>(ctx: &Ctx<'js>, function: &Function<'js>) -> Result<(), EngineError> {
    function
        .set_prototype(Some(&Function::prototype(ctx.clone())))
        .map_err(js_error)
}
