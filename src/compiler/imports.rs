//! OXC-backed module import extraction.

use std::collections::BTreeSet;
use std::path::Path;

use crate::error::EngineError;

mod dynamic;
mod parse;
mod static_requests;

#[cfg(test)]
mod tests;

pub(crate) fn extract_static_import_requests(
    source_text: &str,
    source_path: &Path,
) -> Result<BTreeSet<String>, EngineError> {
    parse::with_module_program(source_text, source_path, |program| {
        dynamic::reject_dynamic_imports(program, source_path)?;
        Ok(static_requests::collect_static_module_requests(program))
    })
}

pub(crate) fn validate_static_module_graph(
    source_text: &str,
    source_path: &Path,
) -> Result<(), EngineError> {
    parse::with_module_program(source_text, source_path, |program| {
        dynamic::reject_dynamic_imports(program, source_path)
    })
}
