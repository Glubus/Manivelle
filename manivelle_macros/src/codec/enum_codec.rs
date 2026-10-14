//! Codec bodies for enums in the four serde representations.
//!
/// Payload decoding lives in one `__manivelle_decode_<Variant>` helper per variant (on an
/// inherent impl), so every representation only decides which JS value holds the payload.
use proc_macro2::TokenStream;
use quote::{format_ident, quote};
use syn::Ident;

use super::field::{decoder, encoder};
use super::shape::{Scope, decode_payload, encode_fields_into, encode_payload};
use crate::model::{Container, Style, Tagging, Variant};

/// Body of `encode_js` for an enum.
pub(super) fn encode_body(container: &Container<'_>, variants: &[Variant<'_>]) -> TokenStream {
    if variants.is_empty() {
        return quote!(match *self {});
    }
    let rust = container.rust_name();
    let arms = variants
        .iter()
        .map(|variant| encode_arm(container, &rust, variant));
    quote!(match self { #(#arms)* })
}

fn encode_arm(container: &Container<'_>, rust: &str, variant: &Variant<'_>) -> TokenStream {
    let ident = variant.ident;
    if variant.attrs.skip_serializing {
        let name = ident.to_string();
        return quote! {
            Self::#ident { .. } => ::manivelle::js::Result::Err(
                ::manivelle::__derive::skipped_variant(#rust, #name)
            ),
        };
    }
    let bindings: Vec<Ident> = (0..variant.shape.fields.len())
        .map(|index| format_ident!("__field{}", index))
        .collect();
    let bound = variant
        .shape
        .fields
        .iter()
        .zip(&bindings)
        .filter(|(field, _)| !field.attrs.skip_serializing)
        .map(|(field, binding)| {
            let member = &field.member;
            quote!(#member: #binding,)
        });
    let accessors: Vec<TokenStream> = bindings.iter().map(|binding| quote!(#binding)).collect();
    let body = encode_variant(container, rust, variant, &accessors);
    quote!(Self::#ident { #(#bound)* .. } => #body,)
}

/// Expression of type `Result<Value>` for one variant in the container's representation.
fn encode_variant(
    container: &Container<'_>,
    rust: &str,
    variant: &Variant<'_>,
    accessors: &[TokenStream],
) -> TokenStream {
    let shape = &variant.shape;
    let key = &variant.name.serialize;
    let scope = variant_scope(container, rust);
    match container.tagging() {
        Tagging::External if shape.style == Style::Unit => {
            quote!(<str as ::manivelle::JsEncode>::encode_js(#key, __ctx))
        }
        Tagging::External => {
            let payload = encode_payload(&scope, shape, accessors);
            quote!({
                let __payload = #payload?;
                let __object = ::manivelle::js::Object::new(__ctx.clone())?;
                __object.set(#key, __payload)?;
                ::manivelle::js::Result::Ok(__object.into_value())
            })
        }
        Tagging::Internal { tag } => {
            let fields = match shape.style {
                Style::Newtype => {
                    let encode = encoder(&shape.fields[0]);
                    let access = &accessors[0];
                    vec![quote! {
                        ::manivelle::__derive::merge_object(&__object, (#encode)(#access, __ctx)?, #rust)?;
                    }]
                }
                _ => encode_fields_into(&scope, shape, accessors),
            };
            tagged_object(tag, key, &fields)
        }
        Tagging::Adjacent { tag, content } => {
            let content = (shape.style != Style::Unit).then(|| {
                let payload = encode_payload(&scope, shape, accessors);
                quote!(__object.set(#content, #payload?)?;)
            });
            tagged_object(tag, key, &Vec::from_iter(content))
        }
        Tagging::Untagged => encode_payload(&scope, shape, accessors),
    }
}

/// `{ [tag]: key, ...statements }` built into `__object`.
fn tagged_object(tag: &str, key: &str, statements: &[TokenStream]) -> TokenStream {
    quote!({
        let __object = ::manivelle::js::Object::new(__ctx.clone())?;
        __object.set(#tag, #key)?;
        #(#statements)*
        ::manivelle::js::Result::Ok(__object.into_value())
    })
}

/// Body of `decode_js` for an enum.
pub(super) fn decode_body(container: &Container<'_>, variants: &[Variant<'_>]) -> TokenStream {
    let rust = container.rust_name();
    let decoded: Vec<&Variant<'_>> = variants
        .iter()
        .filter(|variant| !variant.attrs.skip_deserializing)
        .collect();
    let names = decoded.iter().map(|variant| &variant.name.deserialize);
    let unknown = quote!(::manivelle::__derive::unknown_variant(__other, #rust, &[#(#names),*]));
    match container.tagging() {
        Tagging::External => decode_external(&rust, &decoded, &unknown),
        Tagging::Internal { tag } => decode_internal(&rust, tag, &decoded, &unknown),
        Tagging::Adjacent { tag, content } => {
            decode_adjacent(container, &rust, (tag, content), &decoded, &unknown)
        }
        Tagging::Untagged => decode_untagged(&rust, &decoded),
    }
}

fn decode_external(rust: &str, variants: &[&Variant<'_>], unknown: &TokenStream) -> TokenStream {
    let arms = variants.iter().map(|variant| {
        let pattern = name_pattern(variant);
        let key = &variant.name.deserialize;
        let ident = variant.ident;
        let body = if variant.shape.style == Style::Unit {
            quote!({
                ::manivelle::__derive::unit_payload(__ctx, __payload)
                    .map_err(|__error| ::manivelle::__codec_at_path(__error, #key))?;
                ::manivelle::js::Result::Ok(Self::#ident)
            })
        } else {
            let helper = helper_ident(variant);
            quote! {
                Self::#helper(__ctx, ::manivelle::__derive::variant_payload(__payload, #rust, #key)?)
                    .map_err(|__error| ::manivelle::__codec_at_path(__error, #key))
            }
        };
        quote!(#pattern => #body,)
    });
    quote! {
        let (__variant, __payload) = ::manivelle::__derive::external_variant(__value, #rust)?;
        match __variant.as_str() {
            #(#arms)*
            __other => ::manivelle::js::Result::Err(#unknown),
        }
    }
}

fn decode_internal(
    rust: &str,
    tag: &str,
    variants: &[&Variant<'_>],
    unknown: &TokenStream,
) -> TokenStream {
    let arms = variants.iter().map(|variant| {
        let pattern = name_pattern(variant);
        let ident = variant.ident;
        let helper = helper_ident(variant);
        let body = match variant.shape.style {
            Style::Unit => quote!(::manivelle::js::Result::Ok(Self::#ident)),
            Style::Newtype => quote! {
                Self::#helper(
                    __ctx,
                    ::manivelle::__derive::object_without(__ctx, &__object, &[#tag])?.into_value(),
                )
            },
            Style::Tuple | Style::Struct => quote!(Self::#helper(__ctx, __object.into_value())),
        };
        quote!(#pattern => #body,)
    });
    quote! {
        let __object = ::manivelle::__codec_expect_object(__value, #rust)?;
        let __variant = ::manivelle::__derive::tag_field(&__object, #tag, #rust)?;
        match __variant.as_str() {
            #(#arms)*
            __other => ::manivelle::js::Result::Err(::manivelle::__codec_at_path(#unknown, #tag)),
        }
    }
}

fn decode_adjacent(
    container: &Container<'_>,
    rust: &str,
    (tag, content): (&str, &str),
    variants: &[&Variant<'_>],
    unknown: &TokenStream,
) -> TokenStream {
    let arms = variants.iter().map(|variant| {
        let pattern = name_pattern(variant);
        let ident = variant.ident;
        let body = match variant.shape.style {
            Style::Unit => quote!({
                let __content = ::manivelle::__derive::field_value(&__object, &[#content])?;
                ::manivelle::__derive::unit_payload(__ctx, __content)
                    .map_err(|__error| ::manivelle::__codec_at_path(__error, #content))?;
                ::manivelle::js::Result::Ok(Self::#ident)
            }),
            Style::Newtype => {
                let decode = decoder(&variant.shape.fields[0]);
                quote! {
                    ::manivelle::js::Result::Ok(Self::#ident {
                        0: ::manivelle::__derive::decode_field(__ctx, &__object, &[#content], #decode)?,
                    })
                }
            }
            Style::Tuple | Style::Struct => {
                let helper = helper_ident(variant);
                quote! {
                    Self::#helper(__ctx, ::manivelle::__derive::require_field(&__object, #content, #rust)?)
                        .map_err(|__error| ::manivelle::__codec_at_path(__error, #content))
                }
            }
        };
        quote!(#pattern => #body,)
    });
    let reject_unknown = container.attrs.deny_unknown_fields.then(
        || quote!(::manivelle::__derive::reject_unknown_fields(&__object, &[#tag, #content], #rust)?;),
    );
    quote! {
        let __object = ::manivelle::__codec_expect_object(__value, #rust)?;
        #reject_unknown
        let __variant = ::manivelle::__derive::tag_field(&__object, #tag, #rust)?;
        match __variant.as_str() {
            #(#arms)*
            __other => ::manivelle::js::Result::Err(::manivelle::__codec_at_path(#unknown, #tag)),
        }
    }
}

/// Tries each variant in declaration order; the first that decodes wins.
fn decode_untagged(rust: &str, variants: &[&Variant<'_>]) -> TokenStream {
    let attempts = variants.iter().map(|variant| {
        let ident = variant.ident;
        if variant.shape.style == Style::Unit {
            return quote! {
                if <() as ::manivelle::JsDecode>::decode_js(__ctx, __value.clone()).is_ok() {
                    return ::manivelle::js::Result::Ok(Self::#ident);
                }
            };
        }
        let helper = helper_ident(variant);
        quote! {
            if let ::manivelle::js::Result::Ok(__decoded) = Self::#helper(__ctx, __value.clone()) {
                return ::manivelle::js::Result::Ok(__decoded);
            }
        }
    });
    quote! {
        #(#attempts)*
        ::manivelle::js::Result::Err(::manivelle::__derive::untagged_mismatch(&__value, #rust))
    }
}

/// Per-variant payload decoders used by `decode_body`, one per variant whose payload is
/// decoded as a whole value in this representation.
pub(super) fn decode_helpers(
    container: &Container<'_>,
    variants: &[Variant<'_>],
) -> Vec<TokenStream> {
    let rust = container.rust_name();
    let tagging = container.tagging();
    let scope = Scope {
        tag: match tagging {
            Tagging::Internal { tag } => Some(tag),
            _ => None,
        },
        ..variant_scope(container, &rust)
    };
    variants
        .iter()
        .filter(|variant| {
            !variant.attrs.skip_deserializing && has_helper(&tagging, variant.shape.style)
        })
        .map(|variant| {
            let helper = helper_ident(variant);
            let ident = variant.ident;
            let body = decode_payload(&scope, &variant.shape, &quote!(Self::#ident));
            quote! {
                #[allow(non_snake_case)]
                fn #helper<'__js>(
                    __ctx: &::manivelle::js::Ctx<'__js>,
                    __value: ::manivelle::js::Value<'__js>,
                ) -> ::manivelle::js::Result<Self> {
                    #body
                }
            }
        })
        .collect()
}

fn has_helper(tagging: &Tagging<'_>, style: Style) -> bool {
    match tagging {
        Tagging::External | Tagging::Untagged => style != Style::Unit,
        Tagging::Internal { .. } => matches!(style, Style::Newtype | Style::Struct),
        Tagging::Adjacent { .. } => matches!(style, Style::Tuple | Style::Struct),
    }
}

fn variant_scope<'s>(container: &Container<'_>, rust: &'s str) -> Scope<'s> {
    Scope {
        rust,
        deny_unknown_fields: container.attrs.deny_unknown_fields,
        container_default: None,
        tag: None,
    }
}

fn helper_ident(variant: &Variant<'_>) -> Ident {
    format_ident!("__manivelle_decode_{}", variant.ident)
}

/// `"name" | "alias"` match pattern over the accepted variant names.
fn name_pattern(variant: &Variant<'_>) -> TokenStream {
    let names = variant.name.accepted();
    quote!(#(#names)|*)
}
