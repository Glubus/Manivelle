//! Encoding and decoding of one shape (struct body or enum variant payload): `null` for
//! unit, the inner value for newtypes, an array for tuples and an object for named
//! fields.

use proc_macro2::TokenStream;
use quote::quote;

use super::field::{container_default_value, decoder, default_fn, default_value, encoder};
use crate::attrs::DefaultValue;
use crate::model::{Container, Field, Shape, Style};

/// Facts about the enclosing type that shape code needs.
pub(super) struct Scope<'s> {
    /// Rust type name for conversion errors.
    pub(super) rust: &'s str,
    pub(super) deny_unknown_fields: bool,
    /// The struct's own `#[serde(default)]`; never set for enum variants.
    pub(super) container_default: Option<&'s DefaultValue>,
    /// Internal tag stored next to the fields of this shape.
    pub(super) tag: Option<&'s str>,
}

impl<'s> Scope<'s> {
    /// The scope of the shapes of `container`, without its container default or a tag.
    pub(super) fn of(container: &Container<'_>, rust: &'s str) -> Self {
        Self {
            rust,
            deny_unknown_fields: container.attrs.deny_unknown_fields,
            container_default: None,
            tag: None,
        }
    }
}

/// Expression of type `Result<Value>` encoding the shape; `accessors[i]` is a `&T`
/// expression for field `i`.
pub(super) fn encode_payload(
    scope: &Scope<'_>,
    shape: &Shape<'_>,
    accessors: &[TokenStream],
) -> TokenStream {
    match shape.style {
        Style::Unit => quote! {
            ::std::result::Result::Ok(::manivelle::js::Value::new_null(__ctx.clone()))
        },
        Style::Newtype => {
            let encode = encoder(&shape.fields[0]);
            let access = &accessors[0];
            quote!((#encode)(#access, __ctx))
        }
        Style::Tuple => encode_tuple(shape, accessors),
        Style::Struct => {
            let steps = encode_fields_into(scope, shape, accessors);
            quote!({
                let __object = ::manivelle::js::Object::new(__ctx.clone())?;
                #(#steps)*
                ::std::result::Result::<_, ::manivelle::CodecError>::Ok(__object.into_value())
            })
        }
    }
}

/// Statements writing named fields into `__object`, merging flattened ones.
pub(super) fn encode_fields_into(
    scope: &Scope<'_>,
    shape: &Shape<'_>,
    accessors: &[TokenStream],
) -> Vec<TokenStream> {
    let rust = scope.rust;
    shape
        .fields
        .iter()
        .zip(accessors)
        .filter(|(field, _)| !field.attrs.skip_serializing)
        .map(|(field, access)| {
            let encode = encoder(field);
            let step = if field.attrs.flatten {
                quote! {
                    ::manivelle::__private::derive::merge_object(&__object, (#encode)(#access, __ctx)?, #rust)?;
                }
            } else {
                let key = &field.name.serialize;
                quote! {
                    ::manivelle::__private::derive::encode_field(__ctx, &__object, #key, #access, #encode)?;
                }
            };
            match &field.attrs.skip_serializing_if {
                Some(skip_if) => quote!(if !#skip_if(#access) { #step }),
                None => step,
            }
        })
        .collect()
}

fn encode_tuple(shape: &Shape<'_>, accessors: &[TokenStream]) -> TokenStream {
    let items = shape
        .fields
        .iter()
        .zip(accessors)
        .filter(|(field, _)| !field.attrs.skip_serializing)
        .enumerate()
        .map(|(index, (field, access))| {
            let encode = encoder(field);
            quote!(::manivelle::__private::derive::encode_item(__ctx, &__array, #index, #access, #encode)?;)
        });
    quote!({
        let __array = ::manivelle::js::Array::new(__ctx.clone())?;
        #(#items)*
        ::std::result::Result::<_, ::manivelle::CodecError>::Ok(__array.into_value())
    })
}

/// Expression of type `Result<Self>` decoding `__value` into `constructor`
/// (`Self` or `Self::Variant`).
pub(super) fn decode_payload(
    scope: &Scope<'_>,
    shape: &Shape<'_>,
    constructor: &TokenStream,
) -> TokenStream {
    match shape.style {
        Style::Unit => quote!({
            <() as ::manivelle::JsDecode>::decode_js(__ctx, __value)?;
            ::std::result::Result::Ok(#constructor)
        }),
        Style::Newtype => {
            let field = &shape.fields[0];
            let member = &field.member;
            let decode = decoder(field);
            quote!(::std::result::Result::Ok(#constructor { #member: (#decode)(__ctx, __value)? }))
        }
        Style::Tuple => decode_tuple(scope, shape, constructor),
        Style::Struct => decode_struct(scope, shape, constructor),
    }
}

fn decode_tuple(scope: &Scope<'_>, shape: &Shape<'_>, constructor: &TokenStream) -> TokenStream {
    let rust = scope.rust;
    let mut position = 0_usize;
    let values: Vec<TokenStream> = shape
        .fields
        .iter()
        .map(|field| {
            if field.attrs.skip_deserializing {
                return default_value(field, false);
            }
            let decode = decoder(field);
            let value =
                quote!(::manivelle::__private::derive::decode_item(__ctx, &__array, #position, #decode)?);
            position += 1;
            value
        })
        .collect();
    let members = shape.fields.iter().map(|field| &field.member);
    quote!({
        let __array = ::manivelle::__private::expect_array_len(__value, #rust, #position)?;
        ::std::result::Result::Ok(#constructor { #(#members: #values),* })
    })
}

fn decode_struct(scope: &Scope<'_>, shape: &Shape<'_>, constructor: &TokenStream) -> TokenStream {
    let rust = scope.rust;
    let container_default = scope.container_default.is_some();
    let known = own_keys(scope, shape);
    let reject_unknown = (scope.deny_unknown_fields && !shape.has_flatten()).then(
        || quote!(::manivelle::__private::derive::reject_unknown_fields(&__object, &[#(#known),*], #rust)?;),
    );
    let rest = shape.has_flatten().then(|| {
        quote!(let __rest = ::manivelle::__private::derive::object_without(__ctx, &__object, &[#(#known),*])?;)
    });
    let default = scope.container_default.map(|default| {
        let value = container_default_value(default);
        quote!(let __default: Self = #value;)
    });
    let members = shape.fields.iter().map(|field| &field.member);
    let values = shape
        .fields
        .iter()
        .map(|field| decode_struct_field(field, container_default));
    quote!({
        let __object = ::manivelle::__private::expect_object(__value, #rust)?;
        #reject_unknown
        #rest
        #default
        ::std::result::Result::Ok(#constructor { #(#members: #values),* })
    })
}

/// Keys the shape reads itself (names, aliases and the internal tag). Flattened fields
/// see every other key, like serde's flatten buffer.
fn own_keys<'s>(scope: &Scope<'s>, shape: &'s Shape<'_>) -> Vec<&'s str> {
    shape
        .fields
        .iter()
        .filter(|field| !field.attrs.skip_deserializing && !field.attrs.flatten)
        .flat_map(|field| field.name.accepted())
        .chain(scope.tag)
        .collect()
}

fn decode_struct_field(field: &Field<'_>, container_default: bool) -> TokenStream {
    if field.attrs.skip_deserializing {
        return default_value(field, container_default);
    }
    if field.attrs.flatten {
        return decode_flattened(field);
    }
    let names = field.name.accepted();
    let decode = decoder(field);
    match default_fn(field, container_default) {
        Some(default) => quote! {
            ::manivelle::__private::derive::decode_field_or(__ctx, &__object, &[#(#names),*], #decode, #default)?
        },
        None => quote! {
            ::manivelle::__private::derive::decode_field(__ctx, &__object, &[#(#names),*], #decode)?
        },
    }
}

/// A flattened field decodes from the keys the struct does not claim. A flattened
/// `Option` is `None` when its fields do not decode, as in serde.
fn decode_flattened(field: &Field<'_>) -> TokenStream {
    let native = !field.attrs.json_codec && field.attrs.with.is_none();
    match field.option_inner() {
        Some(inner) if native => quote! {
            <#inner as ::manivelle::JsDecode>::decode_js(__ctx, __rest.clone().into_value()).ok()
        },
        _ => {
            let decode = decoder(field);
            quote!((#decode)(__ctx, __rest.clone().into_value())?)
        }
    }
}
