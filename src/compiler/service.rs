//! TypeScript transpilation through oxc.

use std::mem;
use std::path::Path;
use std::sync::Arc;

use oxc::CompilerInterface;
use oxc::codegen::CodegenReturn;
use oxc::diagnostics::Diagnostics;
use oxc::span::SourceType;
use oxc::transformer::{Module, TransformOptions};

use super::diagnostics::transpile_error;
use super::imports::validate_static_module_graph;
use super::source_map::SourceMap;
use crate::error::EngineError;

/// JavaScript transpiled from one TypeScript module, with its source map.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct TranspiledModule {
    pub(crate) js: String,
    pub(crate) source_map: Arc<SourceMap>,
}

/// TypeScript to JavaScript compiler configured for runtime execution.
#[derive(Debug, Clone)]
pub(crate) struct CompilerService {
    printed: String,
    source_map: SourceMap,
    errors: Diagnostics,
    transform_options: TransformOptions,
}

impl Default for CompilerService {
    fn default() -> Self {
        let mut transform_options = TransformOptions::default();
        transform_options.env.module = Module::Esm;

        Self {
            printed: String::new(),
            source_map: SourceMap::default(),
            errors: Diagnostics::default(),
            transform_options,
        }
    }
}

impl CompilerService {
    /// Compiles one TypeScript source into executable JavaScript. `source_path` names
    /// the file in diagnostics.
    fn execute(
        &mut self,
        source_text: &str,
        source_type: SourceType,
        source_path: &Path,
    ) -> Result<TranspiledModule, EngineError> {
        self.compile(source_text, source_type, source_path);

        if self.errors.is_empty() {
            return Ok(TranspiledModule {
                js: mem::take(&mut self.printed),
                source_map: Arc::new(mem::take(&mut self.source_map)),
            });
        }

        let errors = mem::take(&mut self.errors);
        Err(transpile_error(&errors, source_text, source_path))
    }

    /// Compiles one project module; its source type comes from the file extension.
    pub(crate) fn compile_module(
        &mut self,
        source_text: &str,
        source_path: &Path,
    ) -> Result<TranspiledModule, EngineError> {
        let source_type =
            SourceType::from_path(source_path).map_err(|error| EngineError::Resolve {
                details: error.to_string(),
            })?;
        self.execute(source_text, source_type.with_module(true), source_path)
    }

    /// Compiles one inline TypeScript module; it may only import host modules.
    pub(crate) fn compile_inline(
        &mut self,
        source_text: &str,
        source_path: &Path,
    ) -> Result<TranspiledModule, EngineError> {
        validate_static_module_graph(source_text, source_path)?;
        self.execute(source_text, SourceType::ts().with_module(true), source_path)
    }
}

impl CompilerInterface for CompilerService {
    fn handle_errors(&mut self, errors: Diagnostics) {
        self.errors.extend(errors);
    }

    fn enable_sourcemap(&self) -> bool {
        true
    }

    fn transform_options(&self) -> Option<&TransformOptions> {
        Some(&self.transform_options)
    }

    fn after_codegen(&mut self, ret: CodegenReturn<'_>) {
        self.source_map = ret
            .map
            .map(|map| SourceMap::from_codegen(&map, &ret.code))
            .unwrap_or_default();
        self.printed = ret.code;
    }
}
