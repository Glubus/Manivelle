//! `JsEncode` / `JsDecode` impls that reproduce serde's wire format natively.
//!
/// Both are emitted unless the type restricts them with `#[manivelle(encode_only)]`,
/// `#[manivelle(decode_only)]` or `#[manivelle(schema_only)]`; `#[manivelle(codec = "json")]` on
/// the type routes them through serde_json instead.
mod enum_codec;
mod field;
mod shape;
mod struct_codec;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Generics, parse_quote};

use crate::generics::{bound_type_params, with_predicate};
use crate::model::{Body, Container};

pub(crate) fn expand_codecs(container: &Container<'_>) -> TokenStream {
    let encode = container.codecs().encode.then(|| encode_impl(container));
    let decode = container.codecs().decode.then(|| decode_impl(container));
    quote!(#encode #decode)
}

fn encode_impl(container: &Container<'_>) -> TokenStream {
    let (generics, body) = if container.attrs.json_codec {
        (
            with_predicate(
                container.generics,
                parse_quote!(Self: ::manivelle::__private::serde::Serialize),
            ),
            quote!(::manivelle::__private::derive::json_encode(self, __ctx)),
        )
    } else {
        let body = match &container.body {
            Body::Struct(shape) => struct_codec::encode_body(container, shape),
            Body::Enum(variants) => enum_codec::encode_body(container, variants),
        };
        (
            bound_type_params(container.generics, &parse_quote!(::manivelle::JsEncode)),
            body,
        )
    };
    let ident = container.ident;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics ::manivelle::JsEncode for #ident #ty_generics #where_clause {
            fn encode_js<'__js>(
                &self,
                __ctx: &::manivelle::js::Ctx<'__js>,
            ) -> ::std::result::Result<::manivelle::js::Value<'__js>, ::manivelle::CodecError> {
                #body
            }
        }
    }
}

fn decode_impl(container: &Container<'_>) -> TokenStream {
    if container.attrs.json_codec {
        let generics = with_predicate(
            container.generics,
            parse_quote!(Self: ::manivelle::__private::serde::de::DeserializeOwned),
        );
        return decode_trait_impl(
            container,
            &generics,
            &quote!(::manivelle::__private::derive::json_decode(__ctx, __value)),
        );
    }
    let generics = bound_type_params(container.generics, &parse_quote!(::manivelle::JsDecode));
    match &container.body {
        Body::Struct(shape) => decode_trait_impl(
            container,
            &generics,
            &struct_codec::decode_body(container, shape),
        ),
        Body::Enum(variants) => {
            let decode = decode_trait_impl(
                container,
                &generics,
                &enum_codec::decode_body(container, variants),
            );
            let helpers = decode_helpers_impl(
                container,
                &generics,
                &enum_codec::decode_helpers(container, variants),
            );
            quote!(#decode #helpers)
        }
    }
}

fn decode_trait_impl(
    container: &Container<'_>,
    generics: &Generics,
    body: &TokenStream,
) -> TokenStream {
    let ident = container.ident;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics ::manivelle::JsDecode for #ident #ty_generics #where_clause {
            fn decode_js<'__js>(
                __ctx: &::manivelle::js::Ctx<'__js>,
                __value: ::manivelle::js::Value<'__js>,
            ) -> ::std::result::Result<Self, ::manivelle::CodecError> {
                #body
            }
        }
    }
}

/// Inherent impl holding the per-variant payload decoders.
fn decode_helpers_impl(
    container: &Container<'_>,
    generics: &Generics,
    helpers: &[TokenStream],
) -> TokenStream {
    if helpers.is_empty() {
        return TokenStream::new();
    }
    let ident = container.ident;
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    quote! {
        #[automatically_derived]
        impl #impl_generics #ident #ty_generics #where_clause {
            #(#helpers)*
        }
    }
}
