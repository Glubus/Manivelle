//! `#[derive(TsSchema)]` codecs follow the `serde_json` reference: encoding equals
//! `serde_json::to_string` then `JSON.parse`, decoding equals `JSON.stringify` then
//! `serde_json::from_str`, and the derived schema accepts what serde writes.

#![cfg(feature = "derive")]

mod codecs;
mod enums;
mod harness;
mod structs;
mod types;
