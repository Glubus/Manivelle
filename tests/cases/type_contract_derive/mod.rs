//! `#[derive(TsSchema)]` codecs follow the `serde_json` reference: encoding equals
//! `serde_json::to_string` then `JSON.parse`, decoding equals `JSON.stringify` then
//! `serde_json::from_str`, and the derived schema accepts what serde writes.

mod codecs;
mod enums;
mod structs;
use crate::support;
mod types;
