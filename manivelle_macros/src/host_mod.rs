//! Expansion of `#[host_mod]`: prefixes the `#[host_fn]` names of one module.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Error, Item, ItemFn, ItemMod, LitStr, Meta, parse::Parser, spanned::Spanned};

use crate::host_fn::HostFnArgs;

/// Rewrites every direct `#[host_fn]` of the inline module to carry its full contract
/// name, `namespace.name`, which `#[host_fn]` then expands as usual.
pub(crate) fn expand_host_mod(args: TokenStream, mut module: ItemMod) -> syn::Result<TokenStream> {
    let namespace = parse_namespace(args)?.unwrap_or_else(|| module.ident.to_string());
    let Some((_, items)) = module.content.as_mut() else {
        return Err(Error::new(
            module.ident.span(),
            "`#[host_mod]` needs an inline module (`mod name { ... }`), it cannot see a module file",
        ));
    };

    for item in items {
        if let Item::Fn(function) = item {
            prefix_host_fn(function, &namespace)?;
        }
    }
    Ok(quote!(#module))
}

/// Reads `namespace = "..."`, the only argument.
fn parse_namespace(args: TokenStream) -> syn::Result<Option<String>> {
    let mut namespace = None;
    let parser = syn::meta::parser(|meta| {
        if meta.path.is_ident("namespace") && namespace.is_none() {
            namespace = Some(meta.value()?.parse::<LitStr>()?.value());
            Ok(())
        } else {
            Err(meta.error("expected `namespace = \"name\"`"))
        }
    });
    parser.parse2(args)?;
    Ok(namespace)
}

fn prefix_host_fn(function: &mut ItemFn, namespace: &str) -> syn::Result<()> {
    let fn_name = function.sig.ident.to_string();
    let fn_name = fn_name.trim_start_matches("r#");
    for attr in &mut function.attrs {
        if !is_host_fn(attr) {
            continue;
        }
        let args = match &attr.meta {
            Meta::Path(_) => HostFnArgs::default(),
            Meta::List(list) => HostFnArgs::parse(list.tokens.clone())?,
            Meta::NameValue(_) => {
                return Err(Error::new(attr.span(), "expected `#[host_fn(...)]`"));
            }
        };
        let full = match (args.no_namespace, args.name) {
            (true, Some(name)) => name.value(),
            (true, None) => {
                return Err(Error::new(
                    attr.span(),
                    "`no_namespace` needs `name = \"full.name\"`: it is the full contract name",
                ));
            }
            (false, Some(name)) => format!("{namespace}.{}", name.value()),
            (false, None) => format!("{namespace}.{fn_name}"),
        };
        let path = attr.path();
        *attr = syn::parse_quote!(#[#path(name = #full)]);
    }
    Ok(())
}

fn is_host_fn(attr: &Attribute) -> bool {
    attr.path().segments.last().is_some_and(|segment| {
        segment.ident == "host_fn"
            || segment.ident == "host_fn_with_ctx"
            || segment.ident == "host_fn_async"
    })
}
