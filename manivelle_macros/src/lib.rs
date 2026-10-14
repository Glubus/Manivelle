//! Derive macros for `manivelle`.

mod attrs;
mod check;
mod codec;
mod directions;
mod expand;
mod generics;
mod model;
mod rename;
mod schema;

use proc_macro::TokenStream;
use syn::{Error, parse_macro_input};

use expand::expand_ts_schema;

/// Derives `manivelle::TsSchema` plus the native `manivelle::JsEncode` and `manivelle::JsDecode`
/// codecs.
///
/// The schema and the codecs follow serde's wire format: a value crosses the
/// Rust ↔ JavaScript boundary exactly as `serde_json` would write or read it.
///
/// Shapes: named structs (object), tuple structs (array), newtype structs (inner value),
/// unit structs (`null`), and enums in every serde representation (external, `tag`,
/// `tag` + `content`, `untagged`) with unit, newtype, tuple and struct variants.
/// Generic type parameters are bounded by `TsSchema` / `JsEncode` / `JsDecode`.
///
/// Serde attributes mirrored natively:
///
/// - containers: `rename_all` (also per direction), `rename_all_fields`, `tag`,
///   `content`, `untagged`, `transparent`, `default`, `deny_unknown_fields`
/// - fields: `rename` (also per direction), `alias`, `skip`, `skip_serializing`,
///   `skip_serializing_if = "path"`, `skip_deserializing`, `default`,
///   `default = "path"`, `flatten`
/// - variants: `rename`, `alias`, `rename_all`, `skip`, `skip_serializing`,
///   `skip_deserializing`
///
/// Serde attributes only serde itself can honor (`with`, `serialize_with`,
/// `deserialize_with`, `from`, `try_from`, `into`, `remote`, `bound`, ...) are compile
/// errors until the type or field opts into the `serde_json` reference codec.
///
/// `manivelle` attributes:
///
/// - `#[manivelle(name = "TypeName")]`: TypeScript declaration name.
/// - `#[manivelle(encode_only)]`, `#[manivelle(decode_only)]`, `#[manivelle(schema_only)]` on the
///   type: emit only `JsEncode`, only `JsDecode`, or no codec. A derive macro cannot see
///   which serde traits are derived, so both codecs are emitted by default.
/// - `#[manivelle(codec = "json")]` on the type: encode/decode through `serde_json`.
/// - `#[manivelle(codec = "json")]` on a field: only that field goes through `serde_json`
///   (honoring its serde `with` functions).
/// - `#[manivelle(with = "module")]` on a field: native codec functions
///   `module::encode_js(&value, ctx)` and `module::decode_js(ctx, value)`.
/// - `#[manivelle(type = "TsType")]` on a field: TypeScript type text used instead of the
///   field type's `TsSchema`, which is then not required.
/// - `#[manivelle(rename = "fieldName")]` and `#[manivelle(optional)]`: field overrides for
///   `schema_only` types (elsewhere they would make the schema disagree with the codecs).
///
/// # Panics
///
/// A `#[serde(flatten)]` field or an internally tagged newtype variant merges its type's
/// schema into the surrounding object. Types written as scalars, strings, sequences,
/// tuples, arrays or `HashMap`/`BTreeMap` with integer keys are compile errors; the
/// derive cannot see through other types, so the generated `TsSchema::ts_type()` (and
/// registering a contract that uses the type) panics when:
///
/// - the merged type's schema is not an object, a string-keyed map, `serde_json::Value`
///   or unit: an enum, a newtype over a scalar, an integer-keyed map behind a type alias;
/// - a merged field has the same name as another field of the object;
/// - two flattened maps have different value types.
#[proc_macro_derive(TsSchema, attributes(manivelle, serde))]
pub fn derive_ts_schema(input: TokenStream) -> TokenStream {
    let input = parse_macro_input!(input as syn::DeriveInput);
    expand_ts_schema(&input)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}
