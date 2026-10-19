//! Positions in transpiled JavaScript mapped back to the TypeScript they came from.

use std::borrow::Cow;
use std::sync::Arc;

use oxc_sourcemap::{JSONSourceMap, Token};

/// The one source name of an encoded table: mappings carry positions only, never names.
const PLACEHOLDER_SOURCE: &str = "";

/// Line and column lookup from one transpiled module back to its TypeScript source.
///
/// Positions are 0-based in the table. JavaScript columns are UTF-8 byte offsets, as
/// QuickJS counts them; TypeScript columns are UTF-16 code units, as editors and `tsc`
/// count them.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct SourceMap {
    /// In JavaScript order: codegen emits mappings as it prints the output.
    mappings: Box<[Mapping]>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Mapping {
    js_line: u32,
    js_column: u32,
    ts_line: u32,
    ts_column: u32,
}

/// The TypeScript file one loaded module was transpiled from.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ModuleOrigin {
    /// How errors name the file: `<script id>.ts` for an inline script, the path
    /// relative to the project root for a project module.
    pub(crate) path: String,
    pub(crate) source_map: Arc<SourceMap>,
}

impl SourceMap {
    /// Builds the table from the map oxc codegen produced along with `js`.
    pub(crate) fn from_codegen(map: &oxc_sourcemap::SourceMap<'_>, js: &str) -> Self {
        let mut columns = ByteColumns::new(js);
        let mappings = map
            .get_tokens()
            .map(|token| {
                let js_column = columns.byte_column(token.get_dst_line(), token.get_dst_col());
                Mapping::new(token, js_column)
            })
            .collect();
        Self { mappings }
    }

    /// The table as source map `mappings` text, for the disk cache.
    pub(crate) fn encode(&self) -> String {
        let tokens = self.mappings.iter().map(Mapping::token).collect();
        // One unnamed source: mappings only carry positions. `decode` reads it back under
        // the same name, which is never used.
        oxc_sourcemap::SourceMap::new(
            None,
            Vec::new(),
            None,
            vec![Cow::Borrowed(PLACEHOLDER_SOURCE)],
            Vec::new(),
            tokens,
            None,
        )
        .to_json()
        .mappings
    }

    /// Reads a table [`SourceMap::encode`] wrote; `None` when the text is corrupt.
    pub(crate) fn decode(mappings: &str) -> Option<Self> {
        let map = oxc_sourcemap::SourceMap::from_json(JSONSourceMap {
            version: 3,
            file: None,
            mappings: mappings.to_owned(),
            source_root: None,
            sources: vec![PLACEHOLDER_SOURCE.to_owned()],
            sources_content: None,
            names: Vec::new(),
            debug_id: None,
            x_google_ignore_list: None,
        })
        .ok()?;
        let mappings = map
            .get_tokens()
            .map(|token| Mapping::new(token, token.get_dst_col()))
            .collect();
        Some(Self { mappings })
    }

    /// The 1-based TypeScript line and column of a 1-based JavaScript position, as
    /// QuickJS reports it: the closest mapping at or before it on the same line.
    pub(crate) fn original_position(&self, js_line: u32, js_column: u32) -> Option<(u32, u32)> {
        let position = (js_line.checked_sub(1)?, js_column.checked_sub(1)?);
        let after = self
            .mappings
            .partition_point(|mapping| (mapping.js_line, mapping.js_column) <= position);
        let mapping = self.mappings.get(after.checked_sub(1)?)?;
        (mapping.js_line == position.0).then_some((mapping.ts_line + 1, mapping.ts_column + 1))
    }
}

impl Mapping {
    fn new(token: Token, js_column: u32) -> Self {
        Self {
            js_line: token.get_dst_line(),
            js_column,
            ts_line: token.get_src_line(),
            ts_column: token.get_src_col(),
        }
    }

    fn token(&self) -> Token {
        Token::new(
            self.js_line,
            self.js_column,
            self.ts_line,
            self.ts_column,
            Some(0),
            None,
        )
    }
}

impl ModuleOrigin {
    /// `path:line:column` of the TypeScript behind a 1-based JavaScript position.
    pub(crate) fn locate(&self, js_line: u32, js_column: u32) -> Option<String> {
        let (line, column) = self.source_map.original_position(js_line, js_column)?;
        Some(format!("{}:{line}:{column}", self.path))
    }
}

/// Converts the UTF-16 columns of generated positions, visited in output order, into
/// the UTF-8 byte columns QuickJS reports.
struct ByteColumns<'a> {
    lines: std::str::Split<'a, char>,
    line: u32,
    /// The current line, from the last visited column on.
    rest: &'a str,
    utf16_column: u32,
    byte_column: u32,
}

impl<'a> ByteColumns<'a> {
    fn new(js: &'a str) -> Self {
        let mut lines = js.split('\n');
        let rest = lines.next().unwrap_or_default();
        Self {
            lines,
            line: 0,
            rest,
            utf16_column: 0,
            byte_column: 0,
        }
    }

    fn byte_column(&mut self, line: u32, utf16_column: u32) -> u32 {
        while self.line < line {
            self.rest = self.lines.next().unwrap_or_default();
            self.line += 1;
            self.utf16_column = 0;
            self.byte_column = 0;
        }
        let mut chars = self.rest.chars();
        while self.utf16_column < utf16_column {
            let Some(character) = chars.next() else { break };
            self.utf16_column += character.len_utf16() as u32;
            self.byte_column += character.len_utf8() as u32;
        }
        self.rest = chars.as_str();
        self.byte_column
    }
}
