//! QuickJS types used by [`JsEncode`](crate::JsEncode), [`JsDecode`](crate::JsDecode)
//! and [`JsArgs`](crate::JsArgs).
//!
//! Implement the codec traits with these paths rather than a direct `rquickjs`
//! dependency: they are always the exact version Manivelle is built with, so two copies
//! of `rquickjs` can never meet in one signature.

pub use rquickjs::function::Args;
pub use rquickjs::{
    Array, ArrayBuffer, Context, Ctx, Error, Function, Object, Result, Runtime, String, TypedArray,
    Value,
};
