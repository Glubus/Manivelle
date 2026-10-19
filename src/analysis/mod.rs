//! Reading the TypeScript of a package to check it against the host contracts.
//!
//! [`analyze_project`] finds which host functions and events a project's code uses, and
//! [`check_package`] compares that with the package's manifest and the program's
//! contracts: a misspelled function, a permission that is missing or never used, an export
//! the program calls that the script lacks.
//!
//! The reading is static and best effort. It sees `math.add(...)`, `ctx.on("event", ...)`,
//! the same through the generated SDK, and host modules imported by name; it cannot see
//! `math[name](...)` (it says so) or a name built at runtime. A finding is a warning to fix
//! early; the permissions the engine enforces at runtime are what keeps a package in bounds.

mod check;
mod usage;

use std::collections::{BTreeSet, HashMap};
use std::path::Path;

use crate::compiler::{discover_project, extract_static_import_requests};
use crate::contract::{HostContractAbi, HostContractDescriptor, HostContractKind};
use crate::error::EngineError;

pub use check::{Finding, PackageReport, Severity, add_permissions, check_package};

/// The names of a program's host functions, events and script exports, which the code is
/// read against.
#[derive(Debug, Clone, Default)]
pub struct ContractNames {
    pub(crate) functions: BTreeSet<String>,
    pub(crate) events: BTreeSet<String>,
    /// The first segment of every dotted function name: `math` for `math.add`.
    pub(crate) namespaces: BTreeSet<String>,
    pub(crate) host_modules: BTreeSet<String>,
    /// `(module, export)` to the contract it gives, for `import { highScore } from "game"`.
    pub(crate) by_import: HashMap<(String, String), String>,
    /// The functions the program calls on scripts, registered with `script_export`.
    pub(crate) exports: BTreeSet<String>,
}

impl ContractNames {
    /// The names `descriptors` declare (`engine.registry().descriptors()`).
    #[must_use]
    pub fn from_descriptors(descriptors: &[HostContractDescriptor]) -> Self {
        let mut names = Self::default();
        for descriptor in descriptors {
            match (&descriptor.kind, &descriptor.abi) {
                (HostContractKind::Export, _) | (_, HostContractAbi::Export { .. }) => {
                    names.exports.insert(descriptor.name.clone());
                    continue;
                }
                (HostContractKind::Function, _) => {
                    names.functions.insert(descriptor.name.clone());
                    if let Some((namespace, _)) = descriptor.name.split_once('.') {
                        names.namespaces.insert(namespace.to_owned());
                    }
                }
                (HostContractKind::Callback, _) => {
                    names.events.insert(descriptor.name.clone());
                }
                _ => continue,
            }
            let import = &descriptor.import;
            if !import.module.is_empty() {
                names.host_modules.insert(import.module.clone());
                if let [export] = import.export_path.as_slice() {
                    names.by_import.insert(
                        (import.module.clone(), export.clone()),
                        descriptor.name.clone(),
                    );
                }
            }
        }
        names
    }

    /// Whether `path` is the start of the name of some host function (`math` of
    /// `math.add`): reaching into it with a computed key hides which one is used.
    pub(crate) fn is_namespace_path(&self, path: &[String]) -> bool {
        let prefix = format!("{}.", path.join("."));
        self.functions.iter().any(|name| name.starts_with(&prefix))
    }

    /// Every function and event: what a permission can name.
    pub(crate) fn callable(&self) -> impl Iterator<Item = &String> {
        self.functions.iter().chain(&self.events)
    }
}

/// Where the code reaches a name.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Usage {
    /// The contract (`math.add`), or what was written when it is none.
    pub name: String,
    /// The file, relative to the project's root.
    pub file: String,
    /// The line, from 1.
    pub line: usize,
}

/// What the code of a project uses.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct ScriptUsage {
    /// The host functions it calls or refers to.
    pub functions: Vec<Usage>,
    /// The events it listens to: `ctx.on("user.found", ...)`, `events.user.found(...)`.
    pub events: Vec<Usage>,
    /// Chains under the name of a host namespace that are no host function: `math.ad`.
    pub unknown: Vec<Usage>,
    /// Computed accesses into a host namespace, or `ctx.on` with a name that is not a
    /// literal: the analysis cannot tell which contract they reach.
    pub dynamic: Vec<Usage>,
    /// The names the entry module exports.
    pub exports: BTreeSet<String>,
    /// Whether the entry module re-exports another module (`export * from`), so that
    /// `exports` may be incomplete.
    pub re_exports_all: bool,
}

/// Reads the project that starts at `entry`, and every file it imports, and reports what
/// they use of the contracts in `contracts`. The generated SDK files a project imports are
/// not read as code: what the project does with their bindings is.
///
/// # Errors
///
/// Fails when the project cannot be read: a file is missing or an import does not resolve.
pub fn analyze_project(
    entry: &Path,
    contracts: &ContractNames,
) -> Result<ScriptUsage, EngineError> {
    let mut externals = contracts.host_modules.clone();
    externals.insert("manivelle:env".to_owned());
    let (project, _) = discover_project(entry, &externals, None, &mut |source, path| {
        extract_static_import_requests(source, path)
    })
    .map_err(|failure| failure.error)?;

    let mut usage = ScriptUsage::default();
    for module in &project.modules {
        if usage::is_generated(&module.source, &module.display_path) {
            continue;
        }
        let is_entry = module.module_id == project.entry_module_id;
        usage::analyze_module(
            &module.source,
            &module.display_path,
            is_entry,
            contracts,
            &mut usage,
        );
    }
    Ok(usage)
}

/// Reads one source as a module, for a test or a tool that has no project on disk.
#[must_use]
pub fn analyze_source(source: &str, file: &str, contracts: &ContractNames) -> ScriptUsage {
    let mut usage = ScriptUsage::default();
    usage::analyze_module(source, file, true, contracts, &mut usage);
    usage
}

#[cfg(test)]
mod tests;
