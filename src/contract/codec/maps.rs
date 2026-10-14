//! Maps, which cross as plain JavaScript objects keyed by strings.
//!
//! Integer keys cross as their decimal text, exactly like `serde_json` map keys, so
//! they keep their full range: a key is text, never a rounded number.

use std::collections::{BTreeMap, HashMap};
use std::hash::{BuildHasher, Hash};

use rquickjs::{Ctx, Filter, Object, Result as JsResult, String as JsString, Value as JsValue};

use super::stack_text::StackText;
use super::{JsDecode, JsEncode, at_path, codec_error, define_property, expect_object};

/// Room for the decimal text of any integer key, `-170141183460469231731687303715884105728`.
const INTEGER_KEY_CAPACITY: usize = 40;

/// Key type of a `HashMap` or `BTreeMap` that crosses as a JavaScript object:
/// `String` and the primitive integers, whose keys cross as their decimal text.
///
/// This trait only names the key bound of the map codecs. It is sealed: it cannot be
/// implemented outside Manivelle, and its methods are implementation details.
pub trait MapKey: Sized + sealed::Sealed {
    /// Calls `write` with the key text.
    #[doc(hidden)]
    fn with_text<R>(&self, write: impl FnOnce(&str) -> R) -> R;

    /// Parses key text, returning it back when it is not a valid key.
    #[doc(hidden)]
    fn from_text(text: String) -> Result<Self, String>;

    /// What a valid key looks like, for error messages.
    #[doc(hidden)]
    fn expected() -> &'static str;
}

mod sealed {
    /// Supertrait that keeps [`MapKey`](super::MapKey) from being implemented elsewhere.
    pub trait Sealed {}
}

impl sealed::Sealed for String {}

impl MapKey for String {
    fn with_text<R>(&self, write: impl FnOnce(&str) -> R) -> R {
        write(self)
    }

    fn from_text(text: String) -> Result<Self, String> {
        Ok(text)
    }

    fn expected() -> &'static str {
        "string key"
    }
}

macro_rules! integer_keys {
    ($($ty:ty),+ $(,)?) => {
        $(
            impl sealed::Sealed for $ty {}

            impl MapKey for $ty {
                fn with_text<R>(&self, write: impl FnOnce(&str) -> R) -> R {
                    let text = StackText::<INTEGER_KEY_CAPACITY>::format(format_args!("{self}"))
                        .expect("integer text fits the key buffer");
                    write(text.as_str())
                }

                fn from_text(text: String) -> Result<Self, String> {
                    match is_canonical_integer(&text).then(|| text.parse().ok()).flatten() {
                        Some(key) => Ok(key),
                        None => Err(text),
                    }
                }

                fn expected() -> &'static str {
                    concat!("decimal ", stringify!($ty), " key")
                }
            }
        )+
    };
}

integer_keys!(
    i8, i16, i32, i64, i128, isize, u8, u16, u32, u64, u128, usize
);

/// Whether `text` is an integer as JSON writes it: optional `-`, no `+`, no leading zeros.
fn is_canonical_integer(text: &str) -> bool {
    let digits = text.strip_prefix('-').unwrap_or(text);
    let leading_zero = digits.len() > 1 && digits.starts_with('0');
    !digits.is_empty() && !leading_zero && digits.bytes().all(|byte| byte.is_ascii_digit())
}

fn encode_entries<'js, 'a, K, V>(
    ctx: &Ctx<'js>,
    entries: impl IntoIterator<Item = (&'a K, &'a V)>,
) -> JsResult<JsValue<'js>>
where
    K: MapKey + 'a,
    V: JsEncode + 'a,
{
    let object = Object::new(ctx.clone())?;
    for (key, value) in entries {
        key.with_text(|text| {
            let value = value
                .encode_js(ctx)
                .map_err(|error| at_path(error, format_args!("[{text}]")))?;
            define_property(&object, text, value)
        })?;
    }
    Ok(object.into_value())
}

/// Decodes the own enumerable string-keyed properties of an object. Properties that
/// `JSON.stringify` drops (`undefined`, functions, symbols) are skipped.
fn decode_entries<'js, K, V>(
    ctx: &Ctx<'js>,
    value: JsValue<'js>,
    rust: &'static str,
    mut insert: impl FnMut(K, V),
) -> JsResult<()>
where
    K: MapKey,
    V: JsDecode,
{
    let object = expect_object(value, rust)?;
    for property in object.own_props::<JsString<'js>, JsValue<'js>>(Filter::default()) {
        let (key, value) = property?;
        if is_dropped_by_json(&value) {
            continue;
        }
        let key = key.to_string()?;
        let value =
            V::decode_js(ctx, value).map_err(|error| at_path(error, format_args!("[{key}]")))?;
        insert(decode_key(key, rust)?, value);
    }
    Ok(())
}

fn decode_key<K: MapKey>(text: String, rust: &'static str) -> JsResult<K> {
    K::from_text(text).map_err(|text| {
        at_path(
            codec_error(
                "string",
                rust,
                format!("expected {}, got {text:?}", K::expected()),
            ),
            format_args!("[{text}]"),
        )
    })
}

fn is_dropped_by_json(value: &JsValue<'_>) -> bool {
    value.is_undefined() || value.is_function() || value.is_symbol()
}

impl<K, V, S> JsEncode for HashMap<K, V, S>
where
    K: MapKey,
    V: JsEncode,
{
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> JsResult<JsValue<'js>> {
        encode_entries(ctx, self)
    }
}

impl<K, V, S> JsDecode for HashMap<K, V, S>
where
    K: MapKey + Eq + Hash,
    V: JsDecode,
    S: BuildHasher + Default,
{
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> JsResult<Self> {
        let mut map = HashMap::with_hasher(S::default());
        decode_entries(ctx, value, "HashMap", |key, value| {
            map.insert(key, value);
        })?;
        Ok(map)
    }
}

impl<K, V> JsEncode for BTreeMap<K, V>
where
    K: MapKey,
    V: JsEncode,
{
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> JsResult<JsValue<'js>> {
        encode_entries(ctx, self)
    }
}

impl<K, V> JsDecode for BTreeMap<K, V>
where
    K: MapKey + Ord,
    V: JsDecode,
{
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> JsResult<Self> {
        let mut map = BTreeMap::new();
        decode_entries(ctx, value, "BTreeMap", |key, value| {
            map.insert(key, value);
        })?;
        Ok(map)
    }
}
