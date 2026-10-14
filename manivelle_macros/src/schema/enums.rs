//! Schema of enums in each serde representation.

use proc_macro2::TokenStream;
use quote::quote;

use super::fields::{flatten_step, object_block, object_schema, shape_type, type_ref};
use crate::model::{Container, Style, Tagging, Variant};

pub(super) fn enum_type(container: &Container<'_>, variants: &[Variant<'_>]) -> TokenStream {
    let variants: Vec<&Variant<'_>> = variants
        .iter()
        .filter(|variant| !variant.skipped_in(container.schema_directions()))
        .collect();
    let all_unit = variants
        .iter()
        .all(|variant| variant.shape.style == Style::Unit);
    let described = variants.iter().map(|variant| Described {
        container,
        variant,
        key: variant_key(container, variant),
    });
    match container.tagging() {
        Tagging::External if all_unit => enum_of(None, described.map(|variant| variant.unit())),
        Tagging::External => union(described.map(|variant| variant.external_type())),
        Tagging::Internal { tag } | Tagging::Adjacent { tag, .. } if all_unit => {
            union(described.map(|variant| variant.tag_object(tag)))
        }
        Tagging::Internal { tag } => enum_of(
            Some(tag),
            described.map(|variant| variant.internal_variant()),
        ),
        Tagging::Adjacent { tag, content } => enum_of(
            Some(tag),
            described.map(|variant| variant.adjacent_variant(content)),
        ),
        Tagging::Untagged => union(described.map(|variant| variant.payload_type())),
    }
}

/// `TsType::Enum`, which renders unit variants as string literals and other variants as
/// objects discriminated by `tag`.
fn enum_of(tag: Option<&str>, variants: impl Iterator<Item = TokenStream>) -> TokenStream {
    let tag = match tag {
        Some(tag) => quote!(::core::option::Option::Some(::std::string::String::from(#tag))),
        None => quote!(::core::option::Option::None),
    };
    quote! {
        ::manivelle::TsType::Enum {
            tag: #tag,
            variants: ::std::vec![#(#variants),*],
        }
    }
}

fn union(types: impl Iterator<Item = TokenStream>) -> TokenStream {
    quote!(::manivelle::TsType::Union(::std::vec![#(#types),*]))
}

fn variant_key(container: &Container<'_>, variant: &Variant<'_>) -> String {
    variant
        .attrs
        .schema_rename
        .clone()
        .unwrap_or_else(|| container.schema_key(&variant.name).to_owned())
}

/// A variant with the key the schema shows for it.
struct Described<'c> {
    container: &'c Container<'c>,
    variant: &'c Variant<'c>,
    key: String,
}

impl Described<'_> {
    fn unit(&self) -> TokenStream {
        let key = &self.key;
        quote!(::manivelle::TsEnumVariant::unit(#key))
    }

    fn literal(&self) -> TokenStream {
        let key = &self.key;
        quote!(::manivelle::TsType::Literal(::manivelle::TsLiteral::String(::std::string::String::from(#key))))
    }

    /// Payload as it appears on its own: `null`, the inner type, a tuple or an object.
    fn payload_type(&self) -> TokenStream {
        shape_type(self.container, &self.variant.shape, false)
    }

    /// `"Name"` for unit variants, `{ Name: payload }` otherwise.
    fn external_type(&self) -> TokenStream {
        if self.variant.shape.style == Style::Unit {
            return self.literal();
        }
        let key = &self.key;
        let payload = self.payload_type();
        quote!(::manivelle::TsType::Object(
            ::std::vec![::manivelle::TsField::required(#key, #payload)]
        ))
    }

    /// `{ tag: "Name" }`, for tagged enums made only of unit variants.
    fn tag_object(&self, tag: &str) -> TokenStream {
        let literal = self.literal();
        quote!(::manivelle::TsType::Object(
            ::std::vec![::manivelle::TsField::required(#tag, #literal)]
        ))
    }

    /// Variant whose fields sit next to the tag.
    fn internal_variant(&self) -> TokenStream {
        let key = &self.key;
        let shape = &self.variant.shape;
        let object = match shape.style {
            Style::Unit => return self.unit(),
            Style::Newtype => object_block(&[flatten_step(
                shape.fields[0].ty,
                false,
                "serde internally tagged newtype variants require a TsSchema object or map type",
            )]),
            Style::Tuple | Style::Struct => object_schema(self.container, shape, false),
        };
        quote!((#object).into_variant(#key))
    }

    /// Variant whose payload sits under the `content` key.
    fn adjacent_variant(&self, content: &str) -> TokenStream {
        let key = &self.key;
        let shape = &self.variant.shape;
        let content_field = match shape.style {
            Style::Unit => return self.unit(),
            Style::Newtype if shape.fields[0].option_inner().is_some() => {
                let ty = type_ref(&shape.fields[0]);
                quote!(::manivelle::TsField::optional(#content, #ty))
            }
            _ => {
                let payload = self.payload_type();
                quote!(::manivelle::TsField::required(#content, #payload))
            }
        };
        quote!(::manivelle::TsEnumVariant::payload(#key, ::std::vec![#content_field]))
    }
}
