//! Values that cross as JavaScript strings: text, `char`, paths and IP addresses.

use std::fmt::Display;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use std::str::FromStr;

use rquickjs::{Ctx, Error as JsError, String as JsString, Value as JsValue};

use super::stack_text::StackText;
use super::{CodecError, JsDecode, JsEncode, mismatch};

/// Room for the longest short text encoded without allocating: IP addresses
/// (`ffff:ffff:ffff:ffff:ffff:ffff:255.255.255.255`), UUIDs and date-times.
const SHORT_TEXT_CAPACITY: usize = 64;

impl JsEncode for str {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        Ok(JsString::from_str(ctx.clone(), self)?.into_value())
    }
}

impl JsEncode for String {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        self.as_str().encode_js(ctx)
    }
}

impl JsDecode for String {
    fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_string(value, "String")
    }
}

impl JsEncode for char {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        self.encode_utf8(&mut [0; 4]).encode_js(ctx)
    }
}

impl JsDecode for char {
    fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        let text = decode_string(value, "char")?;
        let mut characters = text.chars();
        match (characters.next(), characters.next()) {
            (Some(character), None) => Ok(character),
            _ => Err(CodecError::from_js(
                "string",
                "char",
                format!(
                    "expected a string of exactly one character, got {} characters",
                    text.chars().count()
                ),
            )),
        }
    }
}

impl JsEncode for Path {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        self.to_str()
            .ok_or_else(|| {
                CodecError::into_js(
                    "Path",
                    "string",
                    "path contains invalid UTF-8 characters".to_owned(),
                )
            })?
            .encode_js(ctx)
    }
}

impl JsEncode for PathBuf {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
        self.as_path().encode_js(ctx)
    }
}

impl JsDecode for PathBuf {
    fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
        decode_string(value, "PathBuf").map(PathBuf::from)
    }
}

macro_rules! address_codecs {
    ($($ty:ty => $expected:literal),+ $(,)?) => {
        $(
            impl JsEncode for $ty {
                fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<JsValue<'js>, CodecError> {
                    encode_display(ctx, self, stringify!($ty))
                }
            }

            impl JsDecode for $ty {
                fn decode_js<'js>(_ctx: &Ctx<'js>, value: JsValue<'js>) -> Result<Self, CodecError> {
                    decode_parsed(value, stringify!($ty), $expected)
                }
            }
        )+
    };
}

address_codecs!(
    IpAddr => "an IP address",
    Ipv4Addr => "an IPv4 address",
    Ipv6Addr => "an IPv6 address",
);

/// Encodes the `Display` text of a short value without allocating.
pub(super) fn encode_display<'js>(
    ctx: &Ctx<'js>,
    value: &impl Display,
    rust: &'static str,
) -> Result<JsValue<'js>, CodecError> {
    let text =
        StackText::<SHORT_TEXT_CAPACITY>::format(format_args!("{value}")).ok_or_else(|| {
            CodecError::into_js(
                rust,
                "string",
                format!("text is longer than {SHORT_TEXT_CAPACITY} bytes"),
            )
        })?;
    text.as_str().encode_js(ctx)
}

/// Reads a JavaScript string as Rust text; lone UTF-16 surrogates are rejected, as
/// `serde_json` rejects them.
fn decode_string(value: JsValue<'_>, rust: &'static str) -> Result<String, CodecError> {
    value
        .try_into_string()
        .map_err(|value| mismatch(&value, rust, "string"))?
        .to_string()
        .map_err(|error| match error {
            JsError::Utf8(_) => CodecError::from_js(
                "string",
                rust,
                "expected Unicode text, got a string with lone surrogates",
            ),
            other => CodecError::from(other),
        })
}

/// Reads a JavaScript string and parses it with `T`'s [`FromStr`], reporting failures
/// as `expected <expected>, got "<text>" (<reason>)`.
pub(super) fn decode_parsed<T>(
    value: JsValue<'_>,
    rust: &'static str,
    expected: &str,
) -> Result<T, CodecError>
where
    T: FromStr,
    T::Err: Display,
{
    let text = decode_string(value, rust)?;
    text.parse().map_err(|error| {
        CodecError::from_js(
            "string",
            rust,
            format!("expected {expected}, got {text:?} ({error})"),
        )
    })
}
