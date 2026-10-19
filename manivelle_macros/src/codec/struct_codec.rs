//! Codec bodies for structs.

use proc_macro2::TokenStream;
use quote::quote;

use super::field::{decoder, default_value, encoder};
use super::shape::{Scope, decode_payload, encode_payload};
use crate::model::{Container, Field, Shape};

/// Body of `encode_js` for a struct.
pub(super) fn encode_body(container: &Container<'_>, shape: &Shape<'_>) -> TokenStream {
    if let Some(field) = container.transparent_field() {
        let encode = encoder(field);
        let member = &field.member;
        return quote!((#encode)(&self.#member, __ctx));
    }
    let accessors: Vec<TokenStream> = shape
        .fields
        .iter()
        .map(|field| {
            let member = &field.member;
            quote!(&self.#member)
        })
        .collect();
    let rust = container.ident.to_string();
    encode_payload(&struct_scope(container, &rust), shape, &accessors)
}

/// Body of `decode_js` for a struct.
pub(super) fn decode_body(container: &Container<'_>, shape: &Shape<'_>) -> TokenStream {
    if let Some(field) = container.transparent_field() {
        return decode_transparent(shape, field);
    }
    let rust = container.ident.to_string();
    decode_payload(&struct_scope(container, &rust), shape, &quote!(Self))
}

/// The scope of the struct body, which carries the struct's own `#[serde(default)]`.
fn struct_scope<'s>(container: &'s Container<'_>, rust: &'s str) -> Scope<'s> {
    Scope {
        container_default: container.attrs.default.as_ref(),
        ..Scope::of(container, rust)
    }
}

/// `#[serde(transparent)]`: the value is the forwarded field; the others take their
/// defaults.
fn decode_transparent(shape: &Shape<'_>, forwarded: &Field<'_>) -> TokenStream {
    let members = shape.fields.iter().map(|field| &field.member);
    let values = shape.fields.iter().map(|field| {
        if std::ptr::eq(field, forwarded) {
            let decode = decoder(field);
            quote!((#decode)(__ctx, __value)?)
        } else {
            default_value(field, false)
        }
    });
    quote!(::manivelle::js::Result::Ok(Self { #(#members: #values),* }))
}
