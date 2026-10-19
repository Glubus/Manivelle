//! TypeScript compilation: transpilation and static ESM project graphs.

mod diagnostics;
mod imports;
mod project;
mod resolver;
mod service;
mod source_map;
mod stamp;

pub(crate) use diagnostics::LineIndex;
pub(crate) use imports::extract_static_import_requests;
pub(crate) use project::{CompiledModule, ProjectFailure, ProjectState, discover_project};
pub(crate) use service::{CompilerService, TranspiledModule};
pub(crate) use source_map::{ModuleOrigin, SourceMap};
pub(crate) use stamp::WatchedFiles;
