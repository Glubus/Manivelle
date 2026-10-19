//! Argument lists for calling JavaScript functions: tuples pass one argument per
//! element, slices and vectors one argument per item.

use rquickjs::function::Args;
use rquickjs::{Ctx, qjs};

use super::super::arity::for_each_tuple;
use super::{CodecError, JsArgs, JsEncode};

/// Encodes argument `index` and appends it, prefixing errors with `arguments[index]`.
fn push_argument<'js, T: JsEncode + ?Sized>(
    ctx: &Ctx<'js>,
    args: &mut Args<'js>,
    index: usize,
    argument: &T,
) -> Result<(), CodecError> {
    let value = argument
        .encode_js(ctx)
        .map_err(|error| error.at(index).at("arguments"))?;
    args.push_arg(value)?;
    Ok(())
}

impl JsArgs for () {
    fn encode_args<'js>(&self, ctx: &Ctx<'js>) -> Result<Args<'js>, CodecError> {
        Ok(Args::new(ctx.clone(), 0))
    }

    fn encode_scalars(&self, _out: &mut [qjs::JSValue]) -> Option<usize> {
        Some(0)
    }
}

macro_rules! tuple_args {
    ($len:literal => $($index:tt $name:ident),+) => {
        impl<$($name: JsEncode),+> JsArgs for ($($name,)+) {
            fn encode_args<'js>(&self, ctx: &Ctx<'js>) -> Result<Args<'js>, CodecError> {
                let mut args = Args::new(ctx.clone(), $len);
                $(push_argument(ctx, &mut args, $index, &self.$index)?;)+
                Ok(args)
            }

            fn encode_scalars(&self, out: &mut [qjs::JSValue]) -> Option<usize> {
                let out = out.get_mut(..$len)?;
                $(out[$index] = self.$index.encode_scalar()?;)+
                Some($len)
            }
        }
    };
}

for_each_tuple!(tuple_args);

impl<T: JsEncode> JsArgs for [T] {
    fn encode_args<'js>(&self, ctx: &Ctx<'js>) -> Result<Args<'js>, CodecError> {
        let mut args = Args::new(ctx.clone(), self.len());
        for (index, argument) in self.iter().enumerate() {
            push_argument(ctx, &mut args, index, argument)?;
        }
        Ok(args)
    }

    fn encode_scalars(&self, out: &mut [qjs::JSValue]) -> Option<usize> {
        let out = out.get_mut(..self.len())?;
        for (slot, argument) in out.iter_mut().zip(self) {
            *slot = argument.encode_scalar()?;
        }
        Some(self.len())
    }
}

impl<T: JsEncode> JsArgs for Vec<T> {
    fn encode_args<'js>(&self, ctx: &Ctx<'js>) -> Result<Args<'js>, CodecError> {
        self.as_slice().encode_args(ctx)
    }

    fn encode_scalars(&self, out: &mut [qjs::JSValue]) -> Option<usize> {
        self.as_slice().encode_scalars(out)
    }
}

impl<A: JsArgs + ?Sized> JsArgs for &A {
    fn encode_args<'js>(&self, ctx: &Ctx<'js>) -> Result<Args<'js>, CodecError> {
        (**self).encode_args(ctx)
    }

    fn encode_scalars(&self, out: &mut [qjs::JSValue]) -> Option<usize> {
        (**self).encode_scalars(out)
    }
}
