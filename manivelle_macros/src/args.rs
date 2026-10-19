//! Parsing of the `key = value` argument lists of the attributes.

use proc_macro2::TokenStream;
use syn::{
    meta::ParseNestedMeta,
    parse::{Parse, Parser},
};

/// Parses an attribute's argument list. `each` consumes one argument and reports whether
/// it knew it; an argument it does not know, or a repeated one, is rejected with `expected`.
pub(crate) fn parse_args(
    args: TokenStream,
    expected: &str,
    mut each: impl FnMut(&ParseNestedMeta<'_>) -> syn::Result<bool>,
) -> syn::Result<()> {
    syn::meta::parser(|meta| {
        if each(&meta)? {
            Ok(())
        } else {
            Err(meta.error(expected))
        }
    })
    .parse2(args)
}

/// Stores the value of the argument `key` in `slot`. `false` when `meta` names another
/// argument or `key` was already given.
pub(crate) fn take_value<T: Parse>(
    meta: &ParseNestedMeta<'_>,
    key: &str,
    slot: &mut Option<T>,
) -> syn::Result<bool> {
    if !meta.path.is_ident(key) || slot.is_some() {
        return Ok(false);
    }
    *slot = Some(meta.value()?.parse()?);
    Ok(true)
}

/// Sets the flag `key`. `false` when `meta` names another argument or the flag is already
/// set.
pub(crate) fn take_flag(meta: &ParseNestedMeta<'_>, key: &str, flag: &mut bool) -> bool {
    if !meta.path.is_ident(key) || *flag {
        return false;
    }
    *flag = true;
    true
}
