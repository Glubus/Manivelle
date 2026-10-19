//! Comparing a package, its code and the program's contracts.

use std::collections::HashSet;
use std::fmt;

use super::{ContractNames, ScriptUsage, Usage, analyze_project};
use crate::contract::HostContractDescriptor;
use crate::error::EngineError;
use crate::packages::{PackageManifest, PermissionSet, closest, permission_matches};

/// How serious a [`Finding`] is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Severity {
    /// The package cannot work as written.
    Error,
    /// Worth fixing, but the package works.
    Warning,
}

/// One thing [`check_package`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Finding {
    /// How serious it is.
    pub severity: Severity,
    /// What is wrong, with the file and line when the code is at fault.
    pub message: String,
}

impl fmt::Display for Finding {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        let label = match self.severity {
            Severity::Error => "error",
            Severity::Warning => "warning",
        };
        write!(formatter, "{label}: {}", self.message)
    }
}

/// The result of [`check_package`].
#[derive(Debug, Clone, Default, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageReport {
    /// What was found, errors and warnings, in the order they were found.
    pub findings: Vec<Finding>,
    /// The contracts the code uses and the permissions do not list, which
    /// [`add_permissions`](crate::packages::add_permissions) can add to the manifest.
    pub missing_permissions: Vec<String>,
}

impl PackageReport {
    /// How many findings are errors.
    #[must_use]
    pub fn errors(&self) -> usize {
        self.findings
            .iter()
            .filter(|finding| finding.severity == Severity::Error)
            .count()
    }

    fn error(&mut self, message: String) {
        self.findings.push(Finding {
            severity: Severity::Error,
            message,
        });
    }

    fn warning(&mut self, message: String) {
        self.findings.push(Finding {
            severity: Severity::Warning,
            message,
        });
    }
}

/// Checks `manifest` and the code of its package against the contracts `descriptors`
/// declare (`engine.registry().descriptors()`):
///
/// - the entry file exists, and the project it starts can be read;
/// - every permission names at least one contract (with the closest name for a typo);
/// - every host function and event the code uses exists and is permitted;
/// - a permission the code never uses, and a function of the host's the program calls on
///   scripts (`script_export`) that the entry module does not export, are warnings;
/// - code the analysis cannot read (`math[name]`) is a warning, since it hides what is used.
///
/// The code is read statically and best effort; see the `analysis` module documentation.
///
/// # Errors
///
/// Fails only when `manifest` has no usable root; a project that cannot be read is a finding.
pub fn check_package(
    manifest: &PackageManifest,
    descriptors: &[HostContractDescriptor],
) -> Result<PackageReport, EngineError> {
    let contracts = ContractNames::from_descriptors(descriptors);
    let allowed = PermissionSet::new(&manifest.permissions);
    let mut report = PackageReport::default();

    check_permission_names(manifest, &contracts, &mut report);

    let entry = manifest.entry_path();
    if !entry.is_file() {
        report.error(format!("the entry {} is not a file", entry.display()));
        return Ok(report);
    }
    match analyze_project(&entry, &contracts) {
        Ok(usage) => check_usage(manifest, &contracts, &allowed, &usage, &mut report),
        Err(error) => report.error(format!("cannot read the project: {error}")),
    }
    Ok(report)
}

fn check_permission_names(
    manifest: &PackageManifest,
    contracts: &ContractNames,
    report: &mut PackageReport,
) {
    let callable: Vec<&String> = contracts.callable().collect();
    for permission in &manifest.permissions {
        if permission_matches(permission, &callable) {
            continue;
        }
        let hint = if permission.ends_with('*') {
            String::new()
        } else {
            suggestion(permission, callable.iter().map(|name| name.as_str()))
        };
        report.error(format!(
            "permission `{permission}` matches no contract of the program{hint}"
        ));
    }
}

/// The hint that names the candidate closest to `name`, when there is one.
fn suggestion<'a>(name: &str, candidates: impl Iterator<Item = &'a str>) -> String {
    closest(name, candidates)
        .map_or_else(String::new, |close| format!(" (did you mean `{close}`?)"))
}

/// What the code uses of the host, as [`check_usage`] reports it.
struct UsageCheck<'a> {
    contracts: &'a ContractNames,
    allowed: &'a PermissionSet,
    report: &'a mut PackageReport,
    /// The names the code uses that the program has, permitted or not.
    used: HashSet<&'a str>,
    /// The names already reported as missing from the permissions, so each is reported once.
    reported: HashSet<&'a str>,
}

impl<'a> UsageCheck<'a> {
    fn functions(&mut self, functions: &'a [Usage]) {
        for function in functions {
            self.used.insert(&function.name);
            if !self.allowed.allows(&function.name) && self.reported.insert(&function.name) {
                self.report.error(format!(
                    "{}:{}: uses `{}`, which its permissions do not list",
                    function.file, function.line, function.name
                ));
                self.report.missing_permissions.push(function.name.clone());
            }
        }
    }

    fn events(&mut self, events: &'a [Usage]) {
        for event in events {
            if !self.contracts.events.contains(&event.name) {
                let hint = suggestion(
                    &event.name,
                    self.contracts.events.iter().map(String::as_str),
                );
                self.report.error(format!(
                    "{}:{}: listens to `{}`, which is no event of the program{hint}",
                    event.file, event.line, event.name
                ));
                continue;
            }
            self.used.insert(&event.name);
            if !self.allowed.allows(&event.name) && self.reported.insert(&event.name) {
                self.report.error(format!(
                    "{}:{}: listens to `{}`, which its permissions do not list",
                    event.file, event.line, event.name
                ));
                self.report.missing_permissions.push(event.name.clone());
            }
        }
    }

    fn unknown_functions(&mut self, unknown: &[Usage]) {
        for call in unknown {
            let hint = suggestion(
                &call.name,
                self.contracts.functions.iter().map(String::as_str),
            );
            self.report.error(format!(
                "{}:{}: `{}` is not a host function{hint}",
                call.file, call.line, call.name
            ));
        }
    }
}

fn check_usage(
    manifest: &PackageManifest,
    contracts: &ContractNames,
    allowed: &PermissionSet,
    usage: &ScriptUsage,
    report: &mut PackageReport,
) {
    let mut check = UsageCheck {
        contracts,
        allowed,
        report,
        used: HashSet::new(),
        reported: HashSet::new(),
    };
    check.functions(&usage.functions);
    check.events(&usage.events);
    check.unknown_functions(&usage.unknown);
    let UsageCheck { report, used, .. } = check;

    for dynamic in &usage.dynamic {
        report.warning(format!(
            "{}:{}: `{}` hides which host contract it reaches; its permissions must be listed by hand",
            dynamic.file, dynamic.line, dynamic.name
        ));
    }
    if usage.dynamic.is_empty() {
        for permission in &manifest.permissions {
            let covers = PermissionSet::new(std::slice::from_ref(permission));
            let matches_something = contracts.callable().any(|name| covers.allows(name));
            if matches_something && !used.iter().any(|name| covers.allows(name)) {
                report.warning(format!(
                    "its permissions list `{permission}`, which the code never uses"
                ));
            }
        }
    }
    if !usage.re_exports_all {
        for export in &contracts.exports {
            if !usage.exports.contains(export) {
                report.warning(format!(
                    "the entry does not export `{export}`, which the program calls on scripts"
                ));
            }
        }
    }
}
