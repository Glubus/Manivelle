//! Native conversion between Rust values and QuickJS values.
//!
//! Reference semantics: a value crosses the boundary exactly as if Rust had serialized
//! it with `serde_json` and JavaScript had called `JSON.parse` on the text, and the
//! reverse (`JSON.stringify`, then `serde_json`). The native path is an optimization
//! of that reference, never a different contract. Deliberate differences, all stricter
//! or more exact than JSON text:
//!
//! - integers outside the JavaScript safe range (±(2^53 − 1)) fail instead of rounding;
//! - non-finite `f32`/`f64` values cross as `NaN`/`Infinity` instead of `null`;
//! - byte types marked for it cross as `Uint8Array`.

mod args;
mod bytes;
mod collections;
pub mod derive;
mod ecosystem;
mod error;
mod field_atoms;
mod js_text;
mod json;
mod maps;
mod number;
mod scalar;
mod stack_text;
mod text;
mod tuples;
mod wrappers;

use std::fmt::{self, Display};

use rquickjs::atom::PredefinedAtom;
use rquickjs::function::Args;
use rquickjs::{Array, Ctx, Error as JsError, Object, Result as JsResult, Value as JsValue, qjs};

pub(crate) use bytes::with_engine_bytes;
pub use error::{CodecError, PathSegment};
pub use maps::MapKey;
pub(crate) use number::exact_integer;

/// Upper bound, in bytes, on memory reserved up front from a length the other side
/// controls; larger collections still decode, growing as items arrive.
const MAX_PREALLOCATED_BYTES: usize = 1 << 20;

/// Capacity to reserve for `len` items of `T` announced by untrusted input.
pub(crate) fn cautious_capacity<T>(len: usize) -> usize {
    len.min(MAX_PREALLOCATED_BYTES / size_of::<T>().max(1))
}

/// Whether `JSON.stringify` drops a property holding `value`: `undefined`, functions and
/// symbols.
pub(crate) fn is_dropped_by_json(value: &JsValue<'_>) -> bool {
    value.is_undefined() || value.is_function() || value.is_symbol()
}

/// Converts a Rust value into a QuickJS value.
///
/// Implemented for primitives, standard containers and every type deriving
/// [`TsSchema`](crate::TsSchema) together with `serde::Serialize`.
pub trait JsEncode {
    /// Builds the JavaScript representation of `self`.
    ///
    /// # Errors
    ///
    /// Fails when the value has no faithful JavaScript representation, such as an
    /// integer outside the safe range, or when QuickJS cannot allocate the result.
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError>;

    /// The QuickJS value of `self` when it is a number or a boolean, which own no
    /// memory: calls into a script use it to skip the reference counting of a wrapped
    /// value. `None`, the default, for every other type, and for a number that does not
    /// cross (an integer outside the safe range), so that [`encode_js`](Self::encode_js)
    /// reports it.
    #[doc(hidden)]
    fn encode_scalar(&self) -> Option<qjs::JSValue> {
        None
    }
}

/// Converts a QuickJS value into a Rust value.
///
/// Implemented for primitives, standard containers and every type deriving
/// [`TsSchema`](crate::TsSchema) together with `serde::Deserialize`.
pub trait JsDecode: Sized {
    /// Reads `value` as `Self`.
    ///
    /// # Errors
    ///
    /// Fails when `value` does not have the shape `serde_json` would accept for
    /// `Self`, or when a number does not fit the target type exactly.
    fn decode_js<'js>(ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError>;

    /// Reads `value`, a QuickJS value the caller owns, when it is a number, a boolean,
    /// `null` or `undefined` that converts to `Self`, without wrapping it. `None`, the
    /// default, when `Self` is not a scalar or the value does not convert: the caller
    /// then goes through [`decode_js`](Self::decode_js), which reports why.
    #[doc(hidden)]
    fn decode_scalar(_value: qjs::JSValue) -> Option<Self> {
        None
    }
}

/// Argument list for calling a JavaScript function from Rust.
///
/// Tuples of [`JsEncode`] values pass one argument per element; `Vec<T>` and slices
/// pass one argument per item.
pub trait JsArgs {
    /// Encodes every argument.
    ///
    /// # Errors
    ///
    /// Fails when one argument cannot be encoded.
    fn encode_args<'js>(&self, ctx: &Ctx<'js>) -> Result<Args<'js>, CodecError>;

    /// Writes every argument to `out` as a QuickJS value and returns how many, when all
    /// of them are scalars ([`JsEncode::encode_scalar`]) and fit in `out`. `None`, the
    /// default, otherwise.
    #[doc(hidden)]
    fn encode_scalars(&self, _out: &mut [qjs::JSValue]) -> Option<usize> {
        None
    }
}

/// Builds the error for a JavaScript value that does not have the shape `rust` needs,
/// reading `expected <shape>, got <value>`.
pub(crate) fn mismatch(value: &JsValue<'_>, rust: &'static str, expected: &str) -> CodecError {
    CodecError::from_js(
        value.type_name(),
        rust,
        format!("expected {expected}, got {}", Got(value)),
    )
}

/// Short description of a JavaScript value for error messages: numbers show their
/// value, anything else its type.
struct Got<'a, 'js>(&'a JsValue<'js>);

impl Display for Got<'_, '_> {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.0.as_number() {
            Some(number) => write!(formatter, "{number}"),
            None => formatter.write_str(self.0.type_name()),
        }
    }
}

/// Reads `value` as a JavaScript array.
pub(crate) fn expect_array<'js>(
    value: JsValue<'js>,
    rust: &'static str,
) -> Result<Array<'js>, CodecError> {
    value
        .try_into_array()
        .map_err(|value| mismatch(&value, rust, "array"))
}

/// Reads `value` as a JavaScript array of exactly `len` items.
#[doc(hidden)]
pub fn expect_array_len<'js>(
    value: JsValue<'js>,
    rust: &'static str,
    len: usize,
) -> Result<Array<'js>, CodecError> {
    let array = expect_array(value, rust)?;
    let actual = array_length(&array, rust)?;
    if actual == len {
        return Ok(array);
    }
    Err(CodecError::from_js(
        "array",
        rust,
        format!("expected array of length {len}, got length {actual}"),
    ))
}

/// Reads `value` as a JavaScript object that is neither an array nor a function.
#[doc(hidden)]
pub fn expect_object<'js>(
    value: JsValue<'js>,
    rust: &'static str,
) -> Result<Object<'js>, CodecError> {
    if value.is_array() || value.is_function() {
        return Err(mismatch(&value, rust, "object"));
    }
    value
        .try_into_object()
        .map_err(|value| mismatch(&value, rust, "object"))
}

/// Length of a JavaScript array. Lengths beyond `i32::MAX` are stored as floats, which
/// `Array::len` panics on; this reads them, and reports anything else as an error.
pub(crate) fn array_length(array: &Array<'_>, rust: &'static str) -> Result<usize, CodecError> {
    let length = array
        .as_object()
        .get::<_, JsValue<'_>>(PredefinedAtom::Length)?;
    let valid = match length.as_int() {
        Some(int) => usize::try_from(int).ok(),
        None => length
            .as_float()
            .and_then(exact_integer)
            .and_then(|wide| usize::try_from(wide).ok()),
    };
    valid.ok_or_else(|| mismatch(&length, rust, "array length"))
}

/// Defines `object[key] = value` as an own enumerable, writable, configurable data
/// property, as `JSON.parse` would. Unlike `Object::set`, it never runs a setter, so a
/// `"__proto__"` key stays an ordinary property instead of replacing the prototype.
pub(crate) fn define_property<'js>(
    object: &Object<'js>,
    key: &str,
    value: JsValue<'js>,
) -> JsResult<()> {
    // SAFETY: the context of `object` is live and `key` is valid UTF-8 of `key.len()`
    // bytes, which QuickJS copies.
    let atom = unsafe {
        qjs::JS_NewAtomLen(
            object.ctx().as_raw().as_ptr(),
            key.as_ptr().cast(),
            key.len() as _,
        )
    };
    define_with_new_atom(object, atom, value)
}

/// [`define_property`] with a key that is a JavaScript value, such as a property name
/// read from another object.
pub(crate) fn define_property_named<'js>(
    object: &Object<'js>,
    key: &JsValue<'js>,
    value: JsValue<'js>,
) -> JsResult<()> {
    // SAFETY: `key` is a live value of the context of `object`; `JS_ValueToAtom` only
    // borrows it.
    let atom = unsafe { qjs::JS_ValueToAtom(object.ctx().as_raw().as_ptr(), key.as_raw()) };
    define_with_new_atom(object, atom, value)
}

/// Defines the property `atom`, a reference just created for it, and releases `atom`.
fn define_with_new_atom<'js>(
    object: &Object<'js>,
    atom: qjs::JSAtom,
    value: JsValue<'js>,
) -> JsResult<()> {
    if atom == qjs::JS_ATOM_NULL {
        return Err(JsError::Exception);
    }
    let defined = define_atom_property(object, atom, value);
    // SAFETY: `atom` is the reference the caller created; nothing else kept it.
    unsafe { qjs::JS_FreeAtom(object.ctx().as_raw().as_ptr(), atom) };
    defined
}

/// [`define_property`] with an atom key, which it only borrows. It goes through the C
/// API directly: `Object::prop` would build and release two `undefined` accessor values,
/// each holding the context, for every property.
pub(crate) fn define_atom_property<'js>(
    object: &Object<'js>,
    atom: qjs::JSAtom,
    value: JsValue<'js>,
) -> JsResult<()> {
    /// A data property, enumerable, writable and configurable, failing with an exception.
    const FLAGS: u32 = qjs::JS_PROP_HAS_VALUE
        | qjs::JS_PROP_HAS_ENUMERABLE
        | qjs::JS_PROP_HAS_WRITABLE
        | qjs::JS_PROP_HAS_CONFIGURABLE
        | qjs::JS_PROP_C_W_E
        | qjs::JS_PROP_THROW;
    let ctx = object.ctx().as_raw().as_ptr();
    // SAFETY: `object` and `value` belong to `ctx`; `JS_DefineProperty` only borrows
    // them and `atom`, which the caller keeps alive, and takes its own reference to
    // `value` for the property. `value` then drops its reference as usual.
    let defined = unsafe {
        qjs::JS_DefineProperty(
            ctx,
            object.as_value().as_raw(),
            atom,
            value.as_raw(),
            qjs::JS_UNDEFINED,
            qjs::JS_UNDEFINED,
            FLAGS as _,
        )
    };
    if defined < 0 {
        return Err(JsError::Exception);
    }
    Ok(())
}

/// Decodes item `index` of `array`, recording `[index]` in errors.
pub(crate) fn decode_item<'js, T: JsDecode>(
    ctx: &Ctx<'js>,
    array: &Array<'js>,
    index: usize,
) -> Result<T, CodecError> {
    let item = array.get::<JsValue<'js>>(index)?;
    T::decode_js(ctx, item).map_err(|error| error.at(index))
}

/// Encodes `item` found at position `index`, recording `[index]` in errors.
pub(crate) fn encode_item<'js, T: JsEncode + ?Sized>(
    ctx: &Ctx<'js>,
    item: &T,
    index: usize,
) -> Result<JsValue<'js>, CodecError> {
    item.encode_js(ctx).map_err(|error| error.at(index))
}
