//! `Option<T>`, references and smart pointers.

use std::rc::Rc;
use std::sync::Arc;

use rquickjs::{Ctx, Value as JsValue};

use super::{CodecError, JsDecode, JsEncode};

/// `None` is `null`; both `null` and `undefined` read back as `None`.
impl<T: JsEncode> JsEncode for Option<T> {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        match self {
            Some(value) => value.encode_js(ctx),
            None => Ok(JsValue::new_null(ctx.clone())),
        }
    }
}

impl<T: JsDecode> JsDecode for Option<T> {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        if value.is_null() || value.is_undefined() {
            Ok(None)
        } else {
            T::decode_js(ctx, value).map(Some)
        }
    }
}

impl<T: JsEncode + ?Sized> JsEncode for &T {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        (**self).encode_js(ctx)
    }

    fn encode_scalar(&self) -> Option<rquickjs::qjs::JSValue> {
        (**self).encode_scalar()
    }
}

macro_rules! transparent_codecs {
    ($($pointer:ident),+) => {
        $(
            impl<T: JsEncode + ?Sized> JsEncode for $pointer<T> {
                fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
                    (**self).encode_js(ctx)
                }
            }

            impl<T: JsDecode> JsDecode for $pointer<T> {
                fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
                    T::decode_js(ctx, value).map($pointer::new)
                }
            }
        )+
    };
}

transparent_codecs!(Box, Arc, Rc);
