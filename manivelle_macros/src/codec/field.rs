//! Per-field codec selection and default values.
//!
//! A field crosses natively through `JsEncode`/`JsDecode`, through serde_json with
/// `#[manivelle(codec = "json")]` (honoring serde `with` functions), or through a custom
/// native module with `#[manivelle(with = "path")]`.
use proc_macro2::TokenStream;
use quote::{quote, quote_spanned};
use syn::spanned::Spanned;

use crate::attrs::DefaultValue;
use crate::model::{Field, Missing};

/// Callable with the shape of `JsEncode::encode_js`: `(&FieldType, &Ctx) -> Result<Value>`.
/// Spanned on the field type so a missing impl is reported on the field.
pub(super) fn encoder(field: &Field<'_>) -> TokenStream {
    let ty = field.ty;
    if let Some(module) = &field.attrs.with {
        return quote!(|__field, __ctx| #module::encode_js(__field, __ctx));
    }
    if !field.attrs.json_codec {
        return quote_spanned!(ty.span()=> <#ty as ::manivelle::JsEncode>::encode_js);
    }
    match &field.attrs.serialize_with {
        Some(serialize) => quote! {
            |__field, __ctx| ::manivelle::__derive::json_encode_with(__field, __ctx, #serialize)
        },
        None => quote_spanned!(ty.span()=> ::manivelle::__derive::json_encode::<#ty>),
    }
}

/// Callable with the shape of `JsDecode::decode_js`: `(&Ctx, Value) -> Result<FieldType>`.
pub(super) fn decoder(field: &Field<'_>) -> TokenStream {
    let ty = field.ty;
    if let Some(module) = &field.attrs.with {
        return quote!(|__ctx, __field| #module::decode_js(__ctx, __field));
    }
    if !field.attrs.json_codec {
        return quote_spanned!(ty.span()=> <#ty as ::manivelle::JsDecode>::decode_js);
    }
    match &field.attrs.deserialize_with {
        Some(deserialize) => quote! {
            |__ctx, __field| ::manivelle::__derive::json_decode_with(__ctx, __field, #deserialize)
        },
        None => quote_spanned!(ty.span()=> ::manivelle::__derive::json_decode::<#ty>),
    }
}

/// Value of a field the input does not provide: its default, the struct default's
/// member, or `Default::default()` (serde's rule for skipped fields).
pub(super) fn default_value(field: &Field<'_>, container_default: bool) -> TokenStream {
    match field.missing(container_default) {
        Missing::Default(DefaultValue::Path(path)) => quote!(#path()),
        Missing::ContainerDefault => {
            let member = &field.member;
            quote!(__default.#member)
        }
        Missing::Default(DefaultValue::Trait) | Missing::Required => {
            quote!(::core::default::Default::default())
        }
    }
}

/// Callable producing the default of a missing field, or `None` when missing is an
/// error.
pub(super) fn default_fn(field: &Field<'_>, container_default: bool) -> Option<TokenStream> {
    match field.missing(container_default) {
        Missing::Default(DefaultValue::Trait) => Some(quote!(::core::default::Default::default)),
        Missing::Default(DefaultValue::Path(path)) => Some(quote!(#path)),
        Missing::ContainerDefault => {
            let member = &field.member;
            Some(quote!(|| __default.#member))
        }
        Missing::Required => None,
    }
}

/// Expression evaluating the struct's own `#[serde(default)]` value.
pub(super) fn container_default_value(default: &DefaultValue) -> TokenStream {
    match default {
        DefaultValue::Trait => quote!(::core::default::Default::default()),
        DefaultValue::Path(path) => quote!(#path()),
    }
}
