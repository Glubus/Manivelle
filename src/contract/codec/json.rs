//! `serde_json::Value`, converted structurally without JSON text.

use rquickjs::{Ctx, Value as JsValue};
use serde_json::Value;

use super::{CodecError, JsDecode, JsEncode};
use crate::contract::{js_to_json, json_to_js};

impl JsEncode for Value {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        json_to_js(ctx, self)
    }
}

impl JsDecode for Value {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        js_to_json(ctx, value)
    }
}
