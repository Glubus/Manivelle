use std::collections::BTreeMap;

use super::*;

fn origin(path: &str) -> ModuleOrigin {
    ModuleOrigin {
        path: path.to_owned(),
        source_map: Default::default(),
    }
}

fn app_modules() -> Vec<CompiledModule> {
    vec![
        CompiledModule {
            module_id: String::from("/app/main.ts"),
            transpiled_js: String::from("import { value } from './dep';"),
            origin: origin("main.ts"),
            resolved_requests: BTreeMap::from([(
                String::from("./dep"),
                String::from("/app/dep.ts"),
            )]),
        },
        CompiledModule {
            module_id: String::from("/app/dep.ts"),
            transpiled_js: String::from("export const value = 1;"),
            origin: origin("dep.ts"),
            resolved_requests: BTreeMap::new(),
        },
    ]
}

#[test]
fn project_graph_uses_runtime_scoped_module_ids() {
    let store = WorkerModuleStore::default();
    let graph = store
        .insert_project("/app/main.ts", app_modules(), 7)
        .expect("insert project graph");

    assert_eq!(graph.entry_module_id, "manivelle://graph/7//app/main.ts");
    assert_eq!(graph.module_ids.len(), 2);
    let resolved = store
        .resolve("manivelle://graph/7//app/main.ts", "./dep")
        .expect("resolve runtime module");
    assert_eq!(resolved, "manivelle://graph/7//app/dep.ts");
}

#[test]
fn removing_graph_clears_sources_and_resolutions() {
    let store = WorkerModuleStore::default();
    let graph = store
        .insert_project("/app/main.ts", app_modules(), 7)
        .expect("insert project graph");

    store
        .remove_modules(&graph.module_ids)
        .expect("remove graph modules");

    let error = store
        .resolve("manivelle://graph/7//app/main.ts", "./dep")
        .expect_err("graph resolution removed");
    assert!(matches!(error, rquickjs::Error::Resolving { .. }));
}

#[test]
fn removing_a_script_keeps_the_host_module_that_shares_its_name() {
    let store = WorkerModuleStore::default();
    store
        .insert_host_modules(BTreeMap::from([(
            String::from("bench"),
            String::from("export const value = 1;"),
        )]))
        .expect("insert host module");
    let graph = store
        .insert_inline("bench", String::from("export {};"), origin("bench.ts"), 0)
        .expect("insert script graph");

    store
        .remove_modules(&graph.module_ids)
        .expect("remove script graph");

    let resolved = store
        .resolve("manivelle://graph/1/other", "bench")
        .expect("host module still resolvable");
    assert_eq!(resolved, "bench");
}
