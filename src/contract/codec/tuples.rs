//! Tuples, which cross as JavaScript arrays of exactly their length.

use rquickjs::{Array, Ctx, Value as JsValue};

use super::super::arity::for_each_tuple;
use super::{CodecError, JsDecode, JsEncode, decode_item, encode_item, expect_array_len};

macro_rules! tuple_codecs {
    ($len:literal => $($index:tt $name:ident),+) => {
        impl<$($name: JsEncode),+> JsEncode for ($($name,)+) {
            fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
                let array = Array::new(ctx.clone())?;
                $(array.set($index, encode_item(ctx, &self.$index, $index)?)?;)+
                Ok(array.into_value())
            }
        }

        impl<$($name: JsDecode),+> JsDecode for ($($name,)+) {
            fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
                let array = expect_array_len(value, "tuple", $len)?;
                Ok(($(decode_item::<$name>(ctx, &array, $index)?,)+))
            }
        }
    };
}

for_each_tuple!(tuple_codecs);
