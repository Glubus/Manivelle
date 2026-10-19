//! Expansion of `#[host_event(name = "...")]`.

use proc_macro2::TokenStream;
use quote::quote;
use syn::{Error, Item, spanned::Spanned};

use crate::host_fn::{HostFnArgs, doc_text};

/// Keeps the struct or enum and makes it the contract of an event whose payload it is:
/// `HostContract`, `HostCallback` and, with `reply = Type`, `HostRequest`.
pub(crate) fn expand_host_event(args: TokenStream, item: &Item) -> syn::Result<TokenStream> {
    let parsed = HostFnArgs::parse(args.clone())?;
    let name = parsed
        .name
        .ok_or_else(|| Error::new(args.span(), "missing `name = \"event.name\"`"))?;
    let (ident, generics, attrs) = match item {
        Item::Struct(item) => (&item.ident, &item.generics, &item.attrs),
        Item::Enum(item) => (&item.ident, &item.generics, &item.attrs),
        other => {
            return Err(Error::new(
                other.span(),
                "`#[host_event]` goes on the struct or enum that is the event's payload",
            ));
        }
    };
    if !generics.params.is_empty() {
        return Err(Error::new(
            generics.span(),
            "an event payload cannot be generic",
        ));
    }

    let docs = doc_text(attrs);
    let docs_const = (!docs.is_empty()).then(|| quote!(const DOCS: &'static str = #docs;));
    let request = parsed.reply.map(|reply| {
        quote! {
            impl ::manivelle::HostRequest for #ident {
                type Reply = #reply;
            }
        }
    });

    Ok(quote! {
        #item

        impl ::manivelle::HostContract for #ident {
            const NAME: &'static str = #name;
            #docs_const

            fn kind() -> ::manivelle::HostContractKind {
                ::manivelle::HostContractKind::Callback
            }
        }

        impl ::manivelle::HostCallback for #ident {
            type Payload = Self;
        }

        #request
    })
}
