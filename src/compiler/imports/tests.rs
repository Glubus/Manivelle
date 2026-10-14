use std::collections::BTreeSet;
use std::path::Path;

use super::extract_static_import_requests;
use crate::error::EngineError;

#[test]
fn extracts_value_import_forms() {
    let requests = extract_static_import_requests(
        r#"
            import def from "./default";
            import * as ns from "./namespace";
            import { named } from "./named";
        "#,
        Path::new("/tmp/main.ts"),
    )
    .expect("extract imports");

    assert_eq!(
        requests,
        BTreeSet::from([
            String::from("./default"),
            String::from("./named"),
            String::from("./namespace"),
        ])
    );
}

#[test]
fn extracts_value_reexport_forms() {
    let requests = extract_static_import_requests(
        r#"
            export { value } from "./named";
            export * from "./all";
            export * as grouped from "./grouped";
        "#,
        Path::new("/tmp/main.ts"),
    )
    .expect("extract reexports");

    assert_eq!(
        requests,
        BTreeSet::from([
            String::from("./all"),
            String::from("./grouped"),
            String::from("./named"),
        ])
    );
}

#[test]
fn ignores_type_only_imports_and_reexports() {
    let requests = extract_static_import_requests(
        r#"
            import type { Input } from "./input";
            export type { Output } from "./output";
            export { type Shape } from "./shape";
        "#,
        Path::new("/tmp/main.ts"),
    )
    .expect("extract type-only module references");

    assert!(requests.is_empty());
}

#[test]
fn rejects_dynamic_imports() {
    let error = extract_static_import_requests(
        r#"
            export async function load() {
                return import("./lazy");
            }
        "#,
        Path::new("/tmp/main.ts"),
    )
    .expect_err("reject dynamic imports");

    assert!(matches!(
        error,
        EngineError::Resolve { details }
            if details.contains("dynamic import() is not supported; use a static import")
    ));
}
