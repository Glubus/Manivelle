//! Fixed-capacity text buffer for formatting short values without allocating.

use std::fmt::{self, Write};

/// Text formatted into an inline buffer of `N` bytes.
pub(super) struct StackText<const N: usize> {
    bytes: [u8; N],
    len: usize,
}

impl<const N: usize> StackText<N> {
    /// Formats `arguments`, or returns `None` when the text needs more than `N` bytes.
    pub(super) fn format(arguments: fmt::Arguments<'_>) -> Option<Self> {
        let mut text = Self {
            bytes: [0; N],
            len: 0,
        };
        text.write_fmt(arguments).ok()?;
        Some(text)
    }

    /// The formatted text. `write_str` appends a whole `str` or nothing, so the bytes
    /// always hold whole UTF-8 strings.
    pub(super) fn as_str(&self) -> &str {
        std::str::from_utf8(&self.bytes[..self.len])
            .expect("only whole UTF-8 strings are written to the buffer")
    }
}

impl<const N: usize> Write for StackText<N> {
    fn write_str(&mut self, text: &str) -> fmt::Result {
        let end = self.len + text.len();
        let target = self.bytes.get_mut(self.len..end).ok_or(fmt::Error)?;
        target.copy_from_slice(text.as_bytes());
        self.len = end;
        Ok(())
    }
}
