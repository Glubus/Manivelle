//! The error of a conversion between Rust and QuickJS, with the path to the value that
//! failed.
//!
//! The path is data: each enclosing field, index or key is pushed as the error unwinds,
//! and the text (`position.x[2]: expected a finite number`) is rendered only where the
//! error leaves the codec: as a [`rquickjs::Error`] for QuickJS, or through [`Display`].

use std::fmt::{self, Display, Write as _};

/// A value that failed to cross the boundary, with the path to where it failed.
#[derive(Debug)]
pub struct CodecError {
    /// Innermost segment first; rendered outermost first.
    path: Vec<PathSegment>,
    cause: Cause,
}

/// Why a value failed to cross the boundary.
#[derive(Debug)]
enum Cause {
    /// A JavaScript value that does not convert to the Rust type `to`.
    FromJs {
        from: &'static str,
        to: &'static str,
        message: Option<String>,
    },
    /// A Rust value that has no faithful JavaScript representation.
    IntoJs {
        from: &'static str,
        to: &'static str,
        message: Option<String>,
    },
    /// A QuickJS failure, such as a pending exception or an allocation failure. It has no
    /// path: it passes through unchanged.
    Engine(rquickjs::Error),
}

/// One step on the path from a converted value to the value that failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PathSegment {
    /// A property name, a tuple struct field or a variant tag, rendered `name`.
    Field(&'static str),
    /// An array or tuple position, rendered `[3]`.
    Index(usize),
    /// A map key, rendered `[key]`.
    Key(String),
}

impl From<&'static str> for PathSegment {
    fn from(name: &'static str) -> Self {
        Self::Field(name)
    }
}

impl From<usize> for PathSegment {
    fn from(index: usize) -> Self {
        Self::Index(index)
    }
}

impl CodecError {
    /// A JavaScript value of kind `from` that does not convert to the Rust type `to`.
    pub fn from_js(from: &'static str, to: &'static str, message: impl Into<String>) -> Self {
        Self {
            path: Vec::new(),
            cause: Cause::FromJs {
                from,
                to,
                message: Some(message.into()),
            },
        }
    }

    /// A Rust value of kind `from` that has no faithful JavaScript representation as `to`.
    pub(crate) fn into_js(
        from: &'static str,
        to: &'static str,
        message: impl Into<String>,
    ) -> Self {
        Self {
            path: Vec::new(),
            cause: Cause::IntoJs {
                from,
                to,
                message: Some(message.into()),
            },
        }
    }

    /// Records that the failure happened inside `segment`, the field, position or key
    /// that encloses the value already described by `self`.
    #[must_use]
    pub fn at(mut self, segment: impl Into<PathSegment>) -> Self {
        self.path.push(segment.into());
        self
    }

    /// The path and the message, as the text QuickJS and [`Display`] show. `None` only
    /// when there is neither a path nor a message.
    fn text(&self) -> Option<String> {
        let (from, to, message) = match &self.cause {
            Cause::FromJs { from, to, message } | Cause::IntoJs { from, to, message } => {
                (*from, *to, message.as_deref())
            }
            Cause::Engine(_) => return None,
        };
        if self.path.is_empty() {
            return message.map(str::to_owned);
        }
        let detail = message.map_or_else(|| format!("expected {to}, got {from}"), str::to_owned);
        Some(format!("{}: {detail}", self.render_path()))
    }

    /// The path in the form `position.x[2]`: fields are joined with `.` except at the
    /// start, indices and keys attach to what precedes them.
    fn render_path(&self) -> String {
        let mut text = String::new();
        for (position, segment) in self.path.iter().rev().enumerate() {
            match segment {
                PathSegment::Field(name) => {
                    if position > 0 {
                        text.push('.');
                    }
                    text.push_str(name);
                }
                PathSegment::Index(index) => {
                    let _ = write!(text, "[{index}]");
                }
                PathSegment::Key(key) => {
                    let _ = write!(text, "[{key}]");
                }
            }
        }
        text
    }
}

impl Display for CodecError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match (&self.cause, self.text()) {
            (Cause::Engine(error), _) => Display::fmt(error, formatter),
            (_, Some(text)) => formatter.write_str(&text),
            (_, None) => Ok(()),
        }
    }
}

impl std::error::Error for CodecError {}

impl From<rquickjs::Error> for CodecError {
    fn from(error: rquickjs::Error) -> Self {
        let cause = match error {
            rquickjs::Error::FromJs { from, to, message } => Cause::FromJs { from, to, message },
            rquickjs::Error::IntoJs { from, to, message } => Cause::IntoJs { from, to, message },
            other => Cause::Engine(other),
        };
        Self {
            path: Vec::new(),
            cause,
        }
    }
}

impl From<CodecError> for rquickjs::Error {
    fn from(error: CodecError) -> Self {
        let message = error.text();
        match error.cause {
            Cause::FromJs { from, to, .. } => rquickjs::Error::FromJs { from, to, message },
            Cause::IntoJs { from, to, .. } => rquickjs::Error::IntoJs { from, to, message },
            Cause::Engine(error) => error,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn leaf() -> CodecError {
        CodecError::from_js("int", "u8", "expected integer in u8 range, got 300")
    }

    #[test]
    fn nested_field_segments_join_with_dots() {
        let error = leaf().at("x").at("position");

        assert_eq!(
            error.to_string(),
            "position.x: expected integer in u8 range, got 300"
        );
    }

    #[test]
    fn index_and_key_segments_attach_without_dots() {
        let error = leaf()
            .at(PathSegment::Key("a b".to_owned()))
            .at(2_usize)
            .at("items");

        assert_eq!(
            error.to_string(),
            "items[2][a b]: expected integer in u8 range, got 300"
        );
    }

    #[test]
    fn field_after_index_joins_with_a_dot() {
        let error = leaf().at("name").at(0_usize);

        assert_eq!(
            error.to_string(),
            "[0].name: expected integer in u8 range, got 300"
        );
    }

    #[test]
    fn leaf_messages_containing_colons_are_left_alone() {
        let error = CodecError::from_js("string", "u8", "invalid type: string, expected u8")
            .at("x")
            .at("position");

        assert_eq!(
            error.to_string(),
            "position.x: invalid type: string, expected u8"
        );
    }

    #[test]
    fn errors_without_message_describe_the_conversion() {
        let error = CodecError {
            path: vec![PathSegment::Field("bytes")],
            cause: Cause::FromJs {
                from: "object",
                to: "Uint8Array",
                message: None,
            },
        };

        assert_eq!(error.to_string(), "bytes: expected Uint8Array, got object");
    }

    #[test]
    fn js_form_keeps_the_rendered_path() {
        let error = rquickjs::Error::from(leaf().at("x"));

        match error {
            rquickjs::Error::FromJs {
                from: "int",
                to: "u8",
                message: Some(message),
            } => assert_eq!(message, "x: expected integer in u8 range, got 300"),
            other => panic!("unexpected error {other:?}"),
        }
    }

    #[test]
    fn engine_errors_pass_through_unchanged() {
        let error = CodecError::from(rquickjs::Error::Exception).at("x");

        assert!(matches!(
            rquickjs::Error::from(error),
            rquickjs::Error::Exception
        ));
    }
}
