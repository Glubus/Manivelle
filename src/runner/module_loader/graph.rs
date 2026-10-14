//! Runtime-scoped ESM graph identifiers.

use std::collections::HashMap;

use crate::compiler::CompiledModule;

/// Start of every script module id: `manivelle://graph/{graph id}/{module id}`.
pub(crate) const RUNTIME_MODULE_PREFIX: &str = "manivelle://graph/";

/// What a script imports to reach its own `ctx`, `console` and timers, and the host
/// functions bound to it. In a context of its own it is one shared module reading the
/// context's globals; in a context group each script gets an instance of its own.
pub(crate) const ENV_SPECIFIER: &str = "manivelle:env";

/// Start of the id of a group script's environment module: `manivelle:env/{graph id}`.
pub(crate) const ENV_MODULE_PREFIX: &str = "manivelle:env/";

/// Start of the id of a group script's instance of a host module:
/// `manivelle:host/{graph id}/{host module}`.
const HOST_INSTANCE_PREFIX: &str = "manivelle:host/";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct RuntimeModuleGraph {
    pub(crate) graph_id: u64,
    pub(crate) entry_module_id: String,
    pub(crate) module_ids: Vec<String>,
}

pub(super) fn env_module_id(graph_id: u64) -> String {
    format!("{ENV_MODULE_PREFIX}{graph_id}")
}

pub(super) fn host_instance_id(graph_id: u64, host_module: &str) -> String {
    format!("{HOST_INSTANCE_PREFIX}{graph_id}/{host_module}")
}

/// The host module a group script's instance id stands for.
pub(super) fn host_instance_module(module_id: &str) -> Option<&str> {
    let rest = module_id.strip_prefix(HOST_INSTANCE_PREFIX)?;
    rest.split_once('/').map(|(_, module)| module)
}

/// The graph a module id belongs to: a script module, or the environment or host
/// instance module made for that graph.
pub(super) fn graph_of(module_id: &str) -> Option<u64> {
    let rest = module_id
        .strip_prefix(RUNTIME_MODULE_PREFIX)
        .or_else(|| module_id.strip_prefix(ENV_MODULE_PREFIX))
        .or_else(|| module_id.strip_prefix(HOST_INSTANCE_PREFIX))?;
    rest.split('/').next()?.parse().ok()
}

pub(super) fn build_runtime_module_id_map(
    graph_id: u64,
    modules: &[CompiledModule],
) -> HashMap<String, String> {
    modules
        .iter()
        .map(|module| {
            (
                module.module_id.clone(),
                runtime_module_id(graph_id, &module.module_id),
            )
        })
        .collect()
}

pub(super) fn runtime_module_id(graph_id: u64, module_id: &str) -> String {
    format!("{RUNTIME_MODULE_PREFIX}{graph_id}/{module_id}")
}
