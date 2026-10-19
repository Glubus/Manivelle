//! Borrowed view of a JavaScript string that is guaranteed to be valid UTF-8.

use rquickjs::{CString, String as JsString};

use super::CodecError;

/// JavaScript string content checked once to be valid UTF-8, read without copying.
///
/// QuickJS encodes lone UTF-16 surrogates as WTF-8, which is not UTF-8, so
/// `rquickjs::CString::as_str` cannot be trusted on script-controlled text.
pub struct JsText<'js> {
    raw: CString<'js>,
}

impl<'js> JsText<'js> {
    /// Reads `string`, rejecting lone surrogates like `serde_json` does.
    ///
    /// # Errors
    ///
    /// Fails when the string is not valid Unicode text.
    pub fn new(string: JsString<'js>, rust: &'static str) -> Result<Self, CodecError> {
        let raw = string.to_cstring()?;
        if std::str::from_utf8(raw_bytes(&raw)).is_err() {
            return Err(CodecError::from_js(
                "string",
                rust,
                "expected Unicode text, got a string with lone surrogates",
            ));
        }
        Ok(Self { raw })
    }

    /// The text.
    pub fn as_str(&self) -> &str {
        // SAFETY: `new` verified these exact bytes are UTF-8, and the QuickJS buffer they
        // point to is immutable and owned by `self.raw` for as long as `self` lives.
        unsafe { std::str::from_utf8_unchecked(raw_bytes(&self.raw)) }
    }
}

fn raw_bytes<'a>(raw: &'a CString<'_>) -> &'a [u8] {
    // SAFETY: `CString` holds a QuickJS-allocated buffer of exactly `len()` bytes, freed
    // only when `raw` drops; the returned slice borrows `raw`, so it cannot outlive it.
    unsafe { std::slice::from_raw_parts(raw.as_ptr().cast::<u8>(), raw.len()) }
}
