//! `glam` vectors, quaternions and matrices cross as flat arrays of numbers, like
//! their `serde` impls: vectors and quaternions component by component (`x, y, z, w`),
//! matrices column by column. Components follow the `f32`, `f64`, `i32` and `u32` rules.

use ::glam::{
    DVec2, DVec3, DVec4, IVec2, IVec3, IVec4, Mat2, Mat3, Mat4, Quat, UVec2, UVec3, UVec4, Vec2,
    Vec3, Vec4,
};
use rquickjs::{Ctx, Value as JsValue};

use super::super::{CodecError, JsDecode, JsEncode, decode_item, expect_array_len};
use crate::contract::{TsSchema, TsType};

macro_rules! glam_codecs {
    ($($ty:ident: [$component:ty; $len:literal] via $to:ident / $from:ident),+ $(,)?) => {
        $(
            impl JsEncode for $ty {
                fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
                    self.$to().encode_js(ctx)
                }
            }

            impl JsDecode for $ty {
                fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
                    decode_components::<$component, $len>(ctx, value, stringify!($ty))
                        .map(|components| $ty::$from(&components))
                }
            }

            impl TsSchema for $ty {
                fn ts_type() -> TsType {
                    TsType::Tuple(vec![TsType::Number; $len])
                }
            }
        )+
    };
}

glam_codecs!(
    Vec2: [f32; 2] via to_array / from_slice,
    Vec3: [f32; 3] via to_array / from_slice,
    Vec4: [f32; 4] via to_array / from_slice,
    IVec2: [i32; 2] via to_array / from_slice,
    IVec3: [i32; 3] via to_array / from_slice,
    IVec4: [i32; 4] via to_array / from_slice,
    UVec2: [u32; 2] via to_array / from_slice,
    UVec3: [u32; 3] via to_array / from_slice,
    UVec4: [u32; 4] via to_array / from_slice,
    DVec2: [f64; 2] via to_array / from_slice,
    DVec3: [f64; 3] via to_array / from_slice,
    DVec4: [f64; 4] via to_array / from_slice,
    Quat: [f32; 4] via to_array / from_slice,
    Mat2: [f32; 4] via to_cols_array / from_cols_array,
    Mat3: [f32; 9] via to_cols_array / from_cols_array,
    Mat4: [f32; 16] via to_cols_array / from_cols_array,
);

/// Decodes a JavaScript array of exactly `N` components, prefixing errors with the
/// component index.
fn decode_components<'js, T, const N: usize>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
    rust: &'static str,
) -> Result<[T; N], CodecError>
where
    T: JsDecode + Copy + Default,
{
    let array = expect_array_len(value, rust, N)?;
    let mut components = [T::default(); N];
    for (index, component) in components.iter_mut().enumerate() {
        *component = decode_item(ctx, &array, index)?;
    }
    Ok(components)
}
