//! TypeScript transpilation through oxc.

use std::path::Path;
use std::sync::Arc;

use oxc::allocator::Allocator;
use oxc::codegen::{Codegen, CodegenOptions};
use oxc::diagnostics::Diagnostics;
use oxc::parser::Parser;
use oxc::semantic::SemanticBuilder;
use oxc::span::SourceType;
use oxc::transformer::{Module, TransformOptions, Transformer};

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
    transform_options: TransformOptions,
}

impl Default for CompilerService {
    fn default() -> Self {
        let mut transform_options = TransformOptions::default();
        transform_options.env.module = Module::Esm;
        Self { transform_options }
    }
}

impl CompilerService {
    /// Compiles one TypeScript source into executable JavaScript. `source_path` names
    /// the file in diagnostics.
    ///
    /// The steps are those of `oxc::CompilerInterface` for a transform with a source map:
    /// parse, semantic analysis, transform, print. Running them here keeps the minifier,
    /// the mangler and the declaration emitter, which that interface links and a script
    /// runtime never uses, out of the build.
    fn execute(
        &self,
        source_text: &str,
        source_type: SourceType,
        source_path: &Path,
    ) -> Result<TranspiledModule, EngineError> {
        let failed = |errors: &Diagnostics| transpile_error(errors, source_text, source_path);
        let allocator = Allocator::default();
        let parsed = Parser::new(&allocator, source_text, source_type).parse();
        let mut errors = parsed.diagnostics;
        let mut program = parsed.program;

        // The transformer about triples the scopes, symbols and references.
        let analysed = SemanticBuilder::new_compiler()
            .with_excess_capacity(2.0)
            .with_enum_eval(true)
            .with_check_syntax_error(true)
            .with_build_nodes(false)
            .build(&program);
        if !analysed.diagnostics.is_empty() {
            errors.extend(analysed.diagnostics);
            return Err(failed(&errors));
        }

        let transformed = Transformer::new(&allocator, source_path, &self.transform_options)
            .build_with_scoping(analysed.semantic.into_scoping(), &mut program);
        errors.extend(transformed.diagnostics);
        if !errors.is_empty() {
            return Err(failed(&errors));
        }

        let printed = Codegen::new()
            .with_options(CodegenOptions {
                source_map_path: Some(source_path.to_path_buf()),
                ..CodegenOptions::default()
            })
            .build(&program);
        let source_map = (printed.map.as_ref())
            .map(|map| SourceMap::from_codegen(map, &printed.code))
            .unwrap_or_default();
        Ok(TranspiledModule {
            js: printed.code,
            source_map: Arc::new(source_map),
        })
    }

    /// Compiles one project module; its source type comes from the file extension.
    pub(crate) fn compile_module(
        &self,
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
        &self,
        source_text: &str,
        source_path: &Path,
    ) -> Result<TranspiledModule, EngineError> {
        validate_static_module_graph(source_text, source_path)?;
        self.execute(source_text, SourceType::ts().with_module(true), source_path)
    }
}
