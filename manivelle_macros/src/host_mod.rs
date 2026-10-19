//! Expansion of `#[host_mod]`: prefixes the `#[host_fn]` and `#[host_event]` names of one
//! module.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Attribute, Error, Item, ItemMod, LitStr, Meta, parse::Parser, spanned::Spanned};

use crate::host_fn::HostFnArgs;

/// Rewrites every direct `#[host_fn]` and `#[host_event]` of the inline module to carry
/// its full contract name, `namespace.name`, which the attribute then expands as usual.
pub(crate) fn expand_host_mod(args: TokenStream, mut module: ItemMod) -> syn::Result<TokenStream> {
    let namespace = parse_namespace(args)?.unwrap_or_else(|| module.ident.to_string());
    let Some((_, items)) = module.content.as_mut() else {
        return Err(Error::new(
            module.ident.span(),
            "`#[host_mod]` needs an inline module (`mod name { ... }`), it cannot see a module file",
        ));
    };

    for item in items {
        match item {
            Item::Fn(function) => {
                let fn_name = function.sig.ident.to_string();
                let fn_name = fn_name.trim_start_matches("r#");
                prefix_attributes(&mut function.attrs, Some(fn_name), &namespace)?;
            }
            // An event has no function to take a default name from.
            Item::Struct(item) => prefix_attributes(&mut item.attrs, None, &namespace)?,
            Item::Enum(item) => prefix_attributes(&mut item.attrs, None, &namespace)?,
            _ => {}
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

/// Gives each contract attribute in `attrs` its full name. `default_name` is the name a
/// bare `#[host_fn]` takes, the function's; an event must give its own.
fn prefix_attributes(
    attrs: &mut [Attribute],
    default_name: Option<&str>,
    namespace: &str,
) -> syn::Result<()> {
    for attr in attrs {
        if !is_contract_attribute(attr) {
            continue;
        }
        let args = match &attr.meta {
            Meta::Path(_) => HostFnArgs::default(),
            Meta::List(list) => HostFnArgs::parse(list.tokens.clone())?,
            Meta::NameValue(_) => {
                return Err(Error::new(attr.span(), "expected `#[host_fn(...)]`"));
            }
        };
        let full = match (args.no_namespace, args.name, default_name) {
            (true, Some(name), _) => name.value(),
            (true, None, _) => {
                return Err(Error::new(
                    attr.span(),
                    "`no_namespace` needs `name = \"full.name\"`: it is the full contract name",
                ));
            }
            (false, Some(name), _) => format!("{namespace}.{}", name.value()),
            (false, None, Some(default)) => format!("{namespace}.{default}"),
            (false, None, None) => {
                return Err(Error::new(
                    attr.span(),
                    "an event takes its name from `name = \"...\"`",
                ));
            }
        };
        let path = attr.path();
        *attr = match args.reply {
            Some(reply) => syn::parse_quote!(#[#path(name = #full, reply = #reply)]),
            None => syn::parse_quote!(#[#path(name = #full)]),
        };
    }
    Ok(())
}

fn is_contract_attribute(attr: &Attribute) -> bool {
    attr.path().segments.last().is_some_and(|segment| {
        segment.ident == "host_fn"
            || segment.ident == "host_fn_with_ctx"
            || segment.ident == "host_fn_async"
            || segment.ident == "host_event"
    })
}
