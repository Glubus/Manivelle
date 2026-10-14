use proc_macro2::TokenStream;
use quote::quote;
use syn::{DeriveInput, Result};

use crate::check::check;
use crate::codec::expand_codecs;
use crate::model::Container;
use crate::schema::expand_schema_impl;

/// Expands `#[derive(TsSchema)]`: the schema impl plus the requested codecs.
pub(crate) fn expand_ts_schema(input: &DeriveInput) -> Result<TokenStream> {
    let container = Container::from_input(input)?;
    check(&container, input)?;
    let schema = expand_schema_impl(&container);
    let codecs = expand_codecs(&container);
    Ok(quote!(#schema #codecs))
}
