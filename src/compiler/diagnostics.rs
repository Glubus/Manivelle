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
    let lines = LineIndex::new(source);
    let details = diagnostics
        .iter()
        .map(|diagnostic| render(diagnostic, &lines, path))
        .collect::<Vec<_>>()
        .join("\n");
    EngineError::Transpile { details }
}

/// The headline, then each label text at its own position and the help, one per
/// indented line.
fn render(diagnostic: &OxcDiagnostic, lines: &LineIndex<'_>, path: &Path) -> String {
    let mut text = headline(diagnostic, lines, path);
    for label in diagnostic.labels.iter() {
        if let Some(label_text) = label.label() {
            let (line, column) = lines.line_column(label.offset());
            let _ = write!(text, "\n  {line}:{column}: {label_text}");
        }
    }
    if let Some(help) = &diagnostic.help {
        let _ = write!(text, "\n  help: {help}");
    }
    text
}

/// `path:line:column: message` at the primary label, else the first one.
fn headline(diagnostic: &OxcDiagnostic, lines: &LineIndex<'_>, path: &Path) -> String {
    let labels = diagnostic.labels.as_slice();
    let primary = labels
        .iter()
        .find(|label| label.primary())
        .or_else(|| labels.first());
    match primary {
        Some(label) => {
            let (line, column) = lines.line_column(label.offset());
            format!("{}:{line}:{column}: {}", path.display(), diagnostic.message)
        }
        None => format!("{}: {}", path.display(), diagnostic.message),
    }
}

/// The 1-based lines and UTF-16 columns of the byte offsets of one source, as editors
/// count them. Built once per source, so each lookup is a binary search.
pub(crate) struct LineIndex<'a> {
    source: &'a str,
    /// The byte offset where each line starts; the first is always 0.
    starts: Vec<usize>,
}

impl<'a> LineIndex<'a> {
    pub(crate) fn new(source: &'a str) -> Self {
        let starts = std::iter::once(0)
            .chain(source.match_indices('\n').map(|(newline, _)| newline + 1))
            .collect();
        Self { source, starts }
    }

    /// The 1-based line of the byte at `offset`.
    pub(crate) fn line_of(&self, offset: usize) -> usize {
        self.starts.partition_point(|&start| start <= offset)
    }

    /// The 1-based line and UTF-16 column of a byte offset. An offset inside a character
    /// counts the characters before it on its line.
    pub(crate) fn line_column(&self, offset: u32) -> (usize, usize) {
        let offset = (offset as usize).min(self.source.len());
        let line = self.line_of(offset);
        let line_start = self.starts[line - 1];
        let before = self.source.get(line_start..offset).unwrap_or_default();
        (line, before.encode_utf16().count() + 1)
    }
}
