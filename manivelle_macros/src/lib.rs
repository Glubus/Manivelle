//! Derive macros for `manivelle`.

mod attrs;
mod check;
mod codec;
mod directions;
mod expand;
mod generics;
mod host_fn;
mod host_mod;
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

/// Declares a host function contract from a plain Rust function.
///
/// ```ignore
/// #[host_fn(name = "user.find")]
/// fn find_user(input: FindUserInput) -> Result<FindUserOutput, EngineError> {
///     Ok(FindUserOutput { display_name: format!("user {}", input.user_id) })
/// }
/// ```
///
/// Keeps the function and generates a marker type named after it in `PascalCase`
/// (`FindUser`) with the same visibility. The marker implements `HostContract` (the
/// given `name`), `HostFunctionSignature` (`Input` is the parameter type, `()` without
/// parameter; `Output` is the `T` of `Result<T, EngineError>`, or the plain return type
/// of a function that cannot fail) and `HostFunction`, so it registers with
/// `registry.function::<FindUser>()`.
///
/// The function must not be `async` or generic, and takes at most one parameter. To
/// receive the calling script, see [`macro@host_fn_with_ctx`]; for a function settled
/// later, [`macro@host_fn_async`].
#[proc_macro_attribute]
pub fn host_fn(args: TokenStream, item: TokenStream) -> TokenStream {
    expand_host_function(args, item, host_fn::Kind::Static)
}

/// Declares a host function that receives the calling script, `manivelle::Caller`.
///
/// ```ignore
/// #[host_fn_with_ctx(name = "user.find")]
/// fn find_user(ctx: &Caller<'_>, input: FindUserInput) -> Result<FindUserOutput, EngineError> {
///     Ok(FindUserOutput { display_name: format!("{} asks {}", ctx.script_id(), input.user_id) })
/// }
///
/// FindUser::register(engine.registry())?;
/// ```
///
/// Like [`macro@host_fn`], it generates the marker (`FindUser`) with `HostContract` and
/// `HostFunctionSignature`. A static `HostFunction` cannot know its caller, so the
/// marker gets a `register(&registry)` function instead, which registers the function
/// with `function_with_caller`. The input is optional (`fn(ctx)`).
#[proc_macro_attribute]
pub fn host_fn_with_ctx(args: TokenStream, item: TokenStream) -> TokenStream {
    expand_host_function(args, item, host_fn::Kind::WithCtx)
}

/// Declares a host function that settles a `Promise` later, through a
/// `manivelle::HostResolver`.
///
/// ```ignore
/// #[host_fn_async(name = "score.lookup")]
/// fn lookup_score(input: PlayerId, resolver: HostResolver<u32>) -> Result<(), EngineError> {
///     pending().push((input, resolver));
///     Ok(())
/// }
///
/// LookupScore::register(engine.registry())?;
/// ```
///
/// The parameters are `[ctx: &Caller<'_>,] input, resolver: HostResolver<Output>`; the
/// `Output` is the resolver's type parameter. The marker gets a `register(&registry)`
/// function that uses `async_function_with` (or `async_function_with_caller` with a
/// `ctx`). The function returns `Result<(), EngineError>` or nothing.
#[proc_macro_attribute]
pub fn host_fn_async(args: TokenStream, item: TokenStream) -> TokenStream {
    expand_host_function(args, item, host_fn::Kind::Async)
}

fn expand_host_function(args: TokenStream, item: TokenStream, kind: host_fn::Kind) -> TokenStream {
    let function = parse_macro_input!(item as syn::ItemFn);
    host_fn::expand_host_fn(args.into(), &function, kind)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}

/// Prefixes the contract names of the `#[host_fn]` functions of an inline module.
///
/// ```ignore
/// #[host_mod(namespace = "math")] // or `#[host_mod]`: the module's name
/// mod math {
///     #[host_fn]                                   // "math.add"
///     pub fn add(input: AddInput) -> Result<AddOutput, EngineError> { /* ... */ }
///
///     #[host_fn(name = "debug.log", no_namespace)] // "debug.log", not prefixed
///     pub fn log(input: LogInput) -> Result<(), EngineError> { /* ... */ }
/// }
/// ```
///
/// Inside the module, `#[host_fn]` without `name` takes the function's name, and a
/// given `name` is prefixed too. `no_namespace` keeps `name` as the full name. Only the
/// module's own functions are rewritten: a nested module needs its own `#[host_mod]`.
/// The module must be inline, since a macro cannot read `mod name;`. Marker types stay
/// in the module and follow the function's visibility, so use `pub fn` to register
/// them from outside (`math::Add`).
#[proc_macro_attribute]
pub fn host_mod(args: TokenStream, item: TokenStream) -> TokenStream {
    let module = parse_macro_input!(item as syn::ItemMod);
    host_mod::expand_host_mod(args.into(), module)
        .unwrap_or_else(Error::into_compile_error)
        .into()
}
