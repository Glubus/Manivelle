//! Every integration test file, one module each, so they share one test binary.

mod builtins;
mod call_scalars;
#[cfg(feature = "cli")]
mod cli_check;
#[cfg(all(feature = "derive", feature = "cli"))]
mod cli_init;
#[cfg(feature = "cli")]
mod cli_run;
mod context_groups_delivery;
mod context_groups_isolation;
mod context_groups_lifecycle;
mod context_groups_security;
mod contract_validation;
mod deferred;
#[cfg(feature = "derive")]
mod derive_schema_enums;
#[cfg(feature = "derive")]
mod derive_schema_flatten;
#[cfg(feature = "derive")]
mod derive_schema_host;
#[cfg(feature = "derive")]
mod derive_schema_shapes;
#[cfg(feature = "derive")]
mod dogfood_usage;
mod engine_async;
mod engine_budget;
mod engine_events;
mod engine_host_functions;
mod engine_memory;
mod engine_options;
mod engine_projects_cache;
mod engine_projects_host;
mod engine_projects_reload;
mod engine_projects_resolution;
mod engine_scripts;
mod error_reports;
mod fault_isolation;
#[cfg(feature = "derive")]
mod host_fn_attr;
mod host_functions;
mod host_promises;
#[cfg(feature = "derive")]
mod host_registry_api;
mod hostile_scripts;
mod hot_reload;
mod hot_state;
mod interrupt;
mod package_permissions;
mod package_reload_permissions;
mod packages;
mod protocol;
mod requests;
#[cfg(feature = "derive")]
mod schema_contract_api;
mod script_export;
mod script_exports_tsc;
mod script_globals;
mod script_isolation;
mod script_quota;
mod source_maps;
#[cfg(feature = "derive")]
mod state_scope;
mod state_snapshot;
mod targeted_delivery;
#[cfg(feature = "derive")]
mod type_contract_derive;
#[cfg(all(
    feature = "derive",
    feature = "uuid",
    feature = "chrono",
    feature = "glam"
))]
mod type_contract_ecosystem;
mod type_contract_std;
mod web_apis;
