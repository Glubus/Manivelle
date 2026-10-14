//! TypeScript diagnostics rendered as `path:line:column: message`.

use std::fmt::Write;
use std::path::Path;

use oxc::diagnostics::OxcDiagnostic;

use crate::error::EngineError;

/// A transpile error listing each diagnostic at its position in `source`.
pub(crate) fn transpile_error(
    diagnostics: &[OxcDiagnostic],
    source: &str,
    path: &Path,
) -> EngineError {
    let details = diagnostics
        .iter()
        .map(|diagnostic| render(diagnostic, source, path))
        .collect::<Vec<_>>()
        .join("\n");
    EngineError::Transpile { details }
}

/// The headline, then each label text at its own position and the help, one per
/// indented line.
fn render(diagnostic: &OxcDiagnostic, source: &str, path: &Path) -> String {
    let mut text = headline(diagnostic, source, path);
    for label in diagnostic.labels.iter() {
        if let Some(label_text) = label.label() {
            let (line, column) = line_column(source, label.offset());
            let _ = write!(text, "\n  {line}:{column}: {label_text}");
        }
    }
    if let Some(help) = &diagnostic.help {
        let _ = write!(text, "\n  help: {help}");
    }
    text
}

/// `path:line:column: message` at the primary label, else the first one.
fn headline(diagnostic: &OxcDiagnostic, source: &str, path: &Path) -> String {
    let labels = diagnostic.labels.as_slice();
    let primary = labels
        .iter()
        .find(|label| label.primary())
        .or_else(|| labels.first());
    match primary {
        Some(label) => {
            let (line, column) = line_column(source, label.offset());
            format!("{}:{line}:{column}: {}", path.display(), diagnostic.message)
        }
        None => format!("{}: {}", path.display(), diagnostic.message),
    }
}

/// 1-based line and UTF-16 column of a byte offset, as editors count them.
fn line_column(source: &str, offset: u32) -> (usize, usize) {
    let before = source.get(..offset as usize).unwrap_or(source);
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let line = before.matches('\n').count() + 1;
    let column = before[line_start..].encode_utf16().count() + 1;
    (line, column)
}
