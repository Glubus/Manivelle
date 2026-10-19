//! `bool` and `()`.

use rquickjs::{Ctx, Value as JsValue, qjs};

use super::{CodecError, JsDecode, JsEncode, mismatch};

impl JsEncode for bool {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        Ok(JsValue::new_bool(ctx.clone(), *self))
    }

    fn encode_scalar(&self) -> Option<qjs::JSValue> {
        Some(if *self { qjs::JS_TRUE } else { qjs::JS_FALSE })
    }
}

impl JsDecode for bool {
    fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        value
            .as_bool()
            .ok_or_else(|| mismatch(&value, "bool", "boolean"))
    }

    fn decode_scalar(value: qjs::JSValue) -> Option<Self> {
        // SAFETY: reading the tag and payload of a value needs no context; the payload
        // read matches the tag.
        unsafe {
            (qjs::JS_VALUE_GET_NORM_TAG(value) == qjs::JS_TAG_BOOL)
                .then(|| qjs::JS_VALUE_GET_BOOL(value))
        }
    }
}

/// `()` is `null`, like `serde_json`; `undefined` is accepted because a function
/// without a return value produces it.
impl JsEncode for () {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        Ok(JsValue::new_null(ctx.clone()))
    }
}

impl JsDecode for () {
    fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        if value.is_null() || value.is_undefined() {
            Ok(())
        } else {
            Err(mismatch(&value, "()", "null"))
        }
    }

    fn decode_scalar(value: qjs::JSValue) -> Option<Self> {
        // SAFETY: reading the tag of a value needs no context.
        let tag = unsafe { qjs::JS_VALUE_GET_NORM_TAG(value) };
        (tag == qjs::JS_TAG_NULL || tag == qjs::JS_TAG_UNDEFINED).then_some(())
    }
}
