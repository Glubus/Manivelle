//! Expansion of `#[script_export(name = "...", args = (A,), output = T)]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Item, LitStr, Type, parse::Parser, spanned::Spanned};

/// Keeps the struct and implements `ScriptExport` for it.
pub(crate) fn expand_script_export(args: TokenStream, item: &Item) -> syn::Result<TokenStream> {
    let Arguments {
        name,
        args: arguments,
        output,
    } = Arguments::parse(args.clone())?;
    let name = name.ok_or_else(|| {
        Error::new(
            args.span(),
            "missing `name = \"exportName\"`, the function the script exports",
        )
    })?;
    let (ident, generics) = match item {
        Item::Struct(item) => (&item.ident, &item.generics),
        other => {
            return Err(Error::new(
                other.span(),
                "`#[script_export]` goes on a struct (`struct Run;`) that names the export",
            ));
        }
    };
    if !generics.params.is_empty() {
        return Err(Error::new(
            generics.span(),
            "a script export cannot be generic",
        ));
    }
    let arguments = arguments.unwrap_or_else(|| syn::parse_quote!(()));
    let output = output.unwrap_or_else(|| syn::parse_quote!(()));

    Ok(quote! {
        #item

        impl ::manivelle::ScriptExport for #ident {
            const NAME: &'static str = #name;
            type Args = #arguments;
            type Output = #output;
        }
    })
}

/// `name`, `args` (a tuple type, `()` by default) and `output` (`()` by default).
struct Arguments {
    name: Option<LitStr>,
    args: Option<Type>,
    output: Option<Type>,
}

impl Arguments {
    fn parse(tokens: TokenStream) -> syn::Result<Self> {
        let mut parsed = Self {
            name: None,
            args: None,
            output: None,
        };
        let parser = syn::meta::parser(|meta| {
            if meta.path.is_ident("name") && parsed.name.is_none() {
                parsed.name = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("args") && parsed.args.is_none() {
                parsed.args = Some(meta.value()?.parse()?);
            } else if meta.path.is_ident("output") && parsed.output.is_none() {
                parsed.output = Some(meta.value()?.parse()?);
            } else {
                return Err(meta.error(
                    "expected `name = \"export\"`, and optionally `args = (A, B)` and `output = T`",
                ));
            }
            Ok(())
        });
        parser.parse2(tokens)?;
        Ok(parsed)
    }
}
