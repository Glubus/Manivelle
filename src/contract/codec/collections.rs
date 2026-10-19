//! Sequences and sets, which cross as JavaScript arrays.

use std::collections::{BTreeSet, HashSet, VecDeque};
use std::hash::{BuildHasher, Hash};

use rquickjs::{Array, Ctx, Value as JsValue};

use super::{
    CodecError, JsDecode, JsEncode, array_length, cautious_capacity, decode_item, encode_item,
    expect_array, expect_array_len,
};

/// Encodes `items` as a JavaScript array, prefixing errors with the item index.
fn encode_items<'js, 'a, T>(
    ctx: &Ctx<'js>,
    items: impl IntoIterator<Item = &'a T>,
) -> Result<JsValue<'js>, CodecError>
where
    T: JsEncode + 'a,
{
    let array = Array::new(ctx.clone())?;
    for (index, item) in items.into_iter().enumerate() {
        array.set(index, encode_item(ctx, item, index)?)?;
    }
    Ok(array.into_value())
}

/// Decodes every item of a JavaScript array into a collection created by `create`,
/// which receives a capacity bounded by [`cautious_capacity`].
pub(super) fn decode_collection<'js, T, C>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
    rust: &'static str,
    create: impl FnOnce(usize) -> C,
    mut insert: impl FnMut(&mut C, T),
) -> Result<C, CodecError>
where
    T: JsDecode,
{
    let array = expect_array(value, rust)?;
    let len = array_length(&array, rust)?;
    let mut collection = create(cautious_capacity::<T>(len));
    for index in 0..len {
        insert(&mut collection, decode_item(ctx, &array, index)?);
    }
    Ok(collection)
}

impl<T: JsEncode> JsEncode for [T] {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        encode_items(ctx, self)
    }
}

impl<T: JsEncode> JsEncode for Vec<T> {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        encode_items(ctx, self)
    }
}

impl<T: JsDecode> JsDecode for Vec<T> {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_collection(ctx, value, "Vec", Vec::with_capacity, Vec::push)
    }
}

impl<T: JsDecode> JsDecode for Box<[T]> {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_collection(ctx, value, "Box<[T]>", Vec::with_capacity, Vec::push)
            .map(Vec::into_boxed_slice)
    }
}

impl<T: JsEncode> JsEncode for VecDeque<T> {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        encode_items(ctx, self)
    }
}

impl<T: JsDecode> JsDecode for VecDeque<T> {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_collection(
            ctx,
            value,
            "VecDeque",
            VecDeque::with_capacity,
            VecDeque::push_back,
        )
    }
}

impl<T: JsEncode, const N: usize> JsEncode for [T; N] {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        encode_items(ctx, self)
    }
}

/// Fixed-size arrays need exactly `N` items, like `serde_json`.
impl<T: JsDecode, const N: usize> JsDecode for [T; N] {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        let array = expect_array_len(value, "[T; N]", N)?;
        let mut failure = None;
        let items: [Option<T>; N] = std::array::from_fn(|index| {
            if failure.is_some() {
                return None;
            }
            decode_item(ctx, &array, index)
                .map_err(|error| failure = Some(error))
                .ok()
        });
        // Without a failure every slot holds an item, so the `expect` cannot fire.
        match failure {
            Some(error) => Err(error),
            None => Ok(items.map(|item| item.expect("every item decodes when none failed"))),
        }
    }
}

/// Sets cross as arrays; duplicate items collapse, as with `serde_json`.
impl<T: JsEncode, S> JsEncode for HashSet<T, S> {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        encode_items(ctx, self)
    }
}

impl<T, S> JsDecode for HashSet<T, S>
where
    T: JsDecode + Eq + Hash,
    S: BuildHasher + Default,
{
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_collection(
            ctx,
            value,
            "HashSet",
            |len| HashSet::with_capacity_and_hasher(len, S::default()),
            |set, item| {
                set.insert(item);
            },
        )
    }
}

impl<T: JsEncode> JsEncode for BTreeSet<T> {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        encode_items(ctx, self)
    }
}

impl<T: JsDecode + Ord> JsDecode for BTreeSet<T> {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_collection(
            ctx,
            value,
            "BTreeSet",
            |_| BTreeSet::new(),
            |set, item| {
                set.insert(item);
            },
        )
    }
}
