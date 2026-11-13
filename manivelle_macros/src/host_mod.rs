//! Expansion of `#[host_mod]`: prefixes the `#[host_fn]` and `#[host_event]` names of one
//! module, and optionally makes them importable from one module name.

use proc_macro2::{Span, TokenStream};
use quote::quote;
use syn::{Attribute, Error, Ident, Item, ItemMod, LitStr, Meta, spanned::Spanned};

use crate::args::{parse_args, take_value};
use crate::host_fn::{HostFnArgs, MOD_IMPORT_KEY, check_import};

/// Rewrites every direct `#[host_fn]` and `#[host_event]` of the inline module to carry
/// its full contract name, `namespace.name`, and the module's `module` when it has one;
/// the attribute then expands as usual.
pub(crate) fn expand_host_mod(args: TokenStream, mut module: ItemMod) -> syn::Result<TokenStream> {
    let args = ModArgs::parse(args)?;
    let namespace = args.namespace.unwrap_or_else(|| module.ident.to_string());
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
                prefix_attributes(
                    &mut function.attrs,
                    Some(fn_name),
                    &namespace,
                    args.import.as_ref(),
                )?;
            }
            // An event has no function to take a default name from.
            Item::Struct(item) => {
                prefix_attributes(&mut item.attrs, None, &namespace, args.import.as_ref())?;
            }
            Item::Enum(item) => {
                prefix_attributes(&mut item.attrs, None, &namespace, args.import.as_ref())?;
            }
            _ => {}
        }
    }
    Ok(quote!(#module))
}

/// The arguments of `#[host_mod]`.
struct ModArgs {
    namespace: Option<String>,
    /// The module scripts import the contracts from.
    import: Option<LitStr>,
}

impl ModArgs {
    /// Reads `namespace = "..."` and `module = "..."`.
    fn parse(args: TokenStream) -> syn::Result<Self> {
        let mut namespace: Option<LitStr> = None;
        let mut import: Option<LitStr> = None;
        parse_args(
            args,
            "expected `namespace = \"name\"` and, optionally, `module = \"app\"`",
            |meta| {
                Ok(take_value(meta, "namespace", &mut namespace)?
                    || take_value(meta, "module", &mut import)?)
            },
        )?;
        if let Some(import) = &import {
            check_import(import)?;
        }
        Ok(Self {
            namespace: namespace.map(|namespace| namespace.value()),
            import,
        })
    }
}

/// Gives each contract attribute in `attrs` its full name, and the import module unless
/// it has its own. `default_name` is the name a bare `#[host_fn]` takes, the function's;
/// an event must give its own.
fn prefix_attributes(
    attrs: &mut [Attribute],
    default_name: Option<&str>,
    namespace: &str,
    import: Option<&LitStr>,
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
        let reply = args.reply.map(|reply| quote!(, reply = #reply));
        let key = Ident::new(MOD_IMPORT_KEY, Span::call_site());
        let import = import.map(|import| quote!(, #key = #import));
        *attr = syn::parse_quote!(#[#path(name = #full #reply #import)]);
    }
    Ok(())
}

fn is_contract_attribute(attr: &Attribute) -> bool {
    attr.path()
        .segments
        .last()
        .is_some_and(|segment| segment.ident == "host_fn" || segment.ident == "host_event")
}
