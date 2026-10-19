//! `TsSchema` impl: the TypeScript shape of the serde wire format.

mod enums;
mod fields;

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Type, parse_quote};

use crate::generics::bound_type_params;
use crate::model::{Body, Container, Shape};

pub(crate) fn expand_schema_impl(container: &Container<'_>) -> TokenStream {
    let ident = container.ident;
    let generics = bound_type_params(container.generics, &parse_quote!(::manivelle::TsSchema));
    let (impl_generics, ty_generics, where_clause) = generics.split_for_impl();
    let schema_name = container
        .attrs
        .schema_name
        .clone()
        .unwrap_or_else(|| ident.to_string());
    let ts_type = ts_type(container);
    let dependencies = dependencies(container);

    quote! {
        #[automatically_derived]
        impl #impl_generics ::manivelle::TsSchema for #ident #ty_generics #where_clause {
            fn schema_name() -> &'static str {
                #schema_name
            }

            fn ts_type() -> ::manivelle::TsType {
                #ts_type
            }

            fn schema_dependencies() -> ::std::vec::Vec<::manivelle::Schema> {
                #dependencies
            }
        }
    }
}

fn ts_type(container: &Container<'_>) -> TokenStream {
    if let Some(wire_type) = container.wire_type() {
        return quote!(::manivelle::__schema_type_ref::<#wire_type>());
    }
    if let Some(field) = container.transparent_field() {
        return fields::type_ref(field);
    }
    match &container.body {
        Body::Struct(shape) => {
            fields::shape_type(container, shape, container.attrs.default.is_some())
        }
        Body::Enum(variants) => enums::enum_type(container, variants),
    }
}

fn dependencies(container: &Container<'_>) -> TokenStream {
    let pushes: Vec<TokenStream> = if let Some(wire_type) = container.wire_type() {
        vec![push_dependency(wire_type)]
    } else if let Some(field) = container.transparent_field() {
        fields::dependency(field).into_iter().collect()
    } else {
        described_shapes(container)
            .into_iter()
            .flat_map(|(shape, container_default)| {
                fields::shape_dependencies(container, shape, container_default)
            })
            .collect()
    };
    if pushes.is_empty() {
        return quote!(::std::vec::Vec::new());
    }
    quote! {
        let mut dependencies = ::std::vec::Vec::new();
        #(#pushes)*
        dependencies
    }
}

/// Shapes the schema shows, each with whether the struct-level `#[serde(default)]`
/// applies to its fields.
fn described_shapes<'c>(container: &'c Container<'_>) -> Vec<(&'c Shape<'c>, bool)> {
    match &container.body {
        Body::Struct(shape) => vec![(shape, container.attrs.default.is_some())],
        Body::Enum(variants) => variants
            .iter()
            .filter(|variant| !variant.skipped_in(container.schema_directions()))
            .map(|variant| (&variant.shape, false))
            .collect(),
    }
}

fn push_dependency(ty: &Type) -> TokenStream {
    quote!(::manivelle::__push_schema_dependency::<#ty>(&mut dependencies);)
}
