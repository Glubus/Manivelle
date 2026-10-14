use std::path::Path;

use oxc::allocator::Allocator;
use oxc::ast::ast::Program;
use oxc::parser::Parser;
use oxc::span::SourceType;

use crate::compiler::diagnostics::transpile_error;
use crate::error::EngineError;

pub(super) fn with_module_program<T>(
    source_text: &str,
    source_path: &Path,
    action: impl FnOnce(&Program<'_>) -> Result<T, EngineError>,
) -> Result<T, EngineError> {
    let source_type = module_source_type(source_path)?;
    let allocator = Allocator::default();
    let parsed = Parser::new(&allocator, source_text, source_type).parse();

    reject_parse_diagnostics(&parsed.diagnostics, source_text, source_path)?;
    action(&parsed.program)
}

fn module_source_type(source_path: &Path) -> Result<SourceType, EngineError> {
    SourceType::from_path(source_path)
        .map_err(|error| EngineError::Resolve {
            details: error.to_string(),
        })
        .map(|source_type| source_type.with_module(true))
}

/// Syntax errors found while scanning imports are TypeScript errors, not resolution
/// failures, so they surface the same way as errors from the transpiler.
fn reject_parse_diagnostics(
    diagnostics: &oxc::diagnostics::Diagnostics,
    source_text: &str,
    source_path: &Path,
) -> Result<(), EngineError> {
    if diagnostics.is_empty() {
        return Ok(());
    }

    Err(transpile_error(diagnostics, source_text, source_path))
}
