use std::path::Path;

use oxc::ast::ast::{ImportExpression, Program};
use oxc::ast_visit::Visit;

use crate::error::EngineError;

pub(super) fn reject_dynamic_imports(
    program: &Program<'_>,
    source_path: &Path,
) -> Result<(), EngineError> {
    let mut detector = DynamicImportDetector::default();
    detector.visit_program(program);

    if detector.found {
        return Err(EngineError::Resolve {
            details: format!(
                "{}: dynamic import() is not supported; use a static import",
                source_path.display()
            ),
        });
    }

    Ok(())
}

#[derive(Debug, Default)]
struct DynamicImportDetector {
    found: bool,
}

impl<'a> Visit<'a> for DynamicImportDetector {
    fn visit_import_expression(&mut self, _it: &ImportExpression<'a>) {
        self.found = true;
    }
}
