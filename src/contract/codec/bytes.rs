//! [`NativeBytes`], which crosses as a `Uint8Array`.

use std::sync::Arc;

use rquickjs::{ArrayBuffer, Ctx, Object, Result as JsResult, TypedArray, Value as JsValue};

use super::collections::decode_collection;
use super::{JsDecode, JsEncode, codec_error, mismatch};
use crate::contract::NativeBytes;

const RUST: &str = "NativeBytes";

/// Encodes as an immutable `Uint8Array` that shares the Rust allocation (no copy).
impl JsEncode for NativeBytes {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> JsResult<JsValue<'js>> {
        let buffer = ArrayBuffer::from_source_immutable(ctx.clone(), self.clone().into_shared())?;
        TypedArray::<u8>::from_arraybuffer(buffer).map(TypedArray::into_value)
    }
}

/// Decodes a `Uint8Array` or `ArrayBuffer` with one copy, or an array of integers in
/// `0..=255` like `serde_json` does for a byte sequence.
impl JsDecode for NativeBytes {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> JsResult<Self> {
        if value.is_array() {
            return decode_collection(ctx, value, RUST, Vec::with_capacity, Vec::push)
                .map(NativeBytes::from);
        }
        let object = value.as_object().ok_or_else(|| not_bytes(&value))?;
        let copied = with_engine_bytes(object, copy_bytes).ok_or_else(|| not_bytes(&value))?;
        copied.ok_or_else(|| codec_error("object", RUST, "expected bytes, got a detached buffer"))
    }
}

/// Runs `read` on the bytes of a `Uint8Array` or `ArrayBuffer`. `None` means `object` is
/// neither, and `Some(None)` means its buffer is detached. The slice must not outlive
/// `read`: JavaScript may detach or rewrite the buffer as soon as it runs again.
pub(crate) fn with_engine_bytes<R>(
    object: &Object<'_>,
    read: impl FnOnce(&[u8]) -> R,
) -> Option<Option<R>> {
    if let Some(view) = object.as_typed_array::<u8>() {
        // SAFETY: `read` finishes before any JavaScript can run again.
        return Some(unsafe { view.as_bytes() }.map(read));
    }
    if let Some(buffer) = object.as_array_buffer() {
        // SAFETY: `read` finishes before any JavaScript can run again.
        return Some(unsafe { buffer.as_bytes() }.map(read));
    }
    None
}

/// Copies engine-owned bytes into Rust memory.
fn copy_bytes(bytes: &[u8]) -> NativeBytes {
    NativeBytes::from_shared(Arc::from(bytes))
}

fn not_bytes(value: &JsValue<'_>) -> rquickjs::Error {
    mismatch(value, RUST, "Uint8Array, ArrayBuffer or array of bytes")
}
