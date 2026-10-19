//! Comparing a package, its code and the program's contracts.

use std::fmt;

use kdl::KdlDocument;

use super::{ContractNames, ScriptUsage, analyze_project};
use crate::contract::HostContractDescriptor;
use crate::error::EngineError;
use crate::packages::{PackageManifest, PermissionSet};

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
    /// [`add_permissions`] can add to the manifest.
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
/// The code is read statically and best effort; see the [module documentation](self).
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
        let mut message = format!("permission `{permission}` matches no contract of the program");
        if !permission.ends_with('*')
            && let Some(close) = closest(permission, callable.iter().map(|name| name.as_str()))
        {
            message.push_str(&format!(" (did you mean `{close}`?)"));
        }
        report.error(message);
    }
}

fn check_usage(
    manifest: &PackageManifest,
    contracts: &ContractNames,
    allowed: &PermissionSet,
    usage: &ScriptUsage,
    report: &mut PackageReport,
) {
    let mut seen: Vec<&str> = Vec::new();
    let mut used: Vec<&str> = Vec::new();

    for function in &usage.functions {
        used.push(&function.name);
        if !allowed.allows(&function.name) && !seen.contains(&function.name.as_str()) {
            seen.push(&function.name);
            report.error(format!(
                "{}:{}: uses `{}`, which its permissions do not list",
                function.file, function.line, function.name
            ));
            report.missing_permissions.push(function.name.clone());
        }
    }
    for event in &usage.events {
        if !contracts.events.contains(&event.name) {
            let mut message = format!(
                "{}:{}: listens to `{}`, which is no event of the program",
                event.file, event.line, event.name
            );
            if let Some(close) = closest(&event.name, contracts.events.iter().map(String::as_str)) {
                message.push_str(&format!(" (did you mean `{close}`?)"));
            }
            report.error(message);
            continue;
        }
        used.push(&event.name);
        if !allowed.allows(&event.name) && !seen.contains(&event.name.as_str()) {
            seen.push(&event.name);
            report.error(format!(
                "{}:{}: listens to `{}`, which its permissions do not list",
                event.file, event.line, event.name
            ));
            report.missing_permissions.push(event.name.clone());
        }
    }
    for unknown in &usage.unknown {
        let mut message = format!(
            "{}:{}: `{}` is not a host function",
            unknown.file, unknown.line, unknown.name
        );
        if let Some(close) = closest(
            &unknown.name,
            contracts.functions.iter().map(String::as_str),
        ) {
            message.push_str(&format!(" (did you mean `{close}`?)"));
        }
        report.error(message);
    }
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

/// Whether `permission` (a name, `namespace.*` or `*`) covers at least one of `contracts`.
pub(crate) fn permission_matches(permission: &str, contracts: &[&String]) -> bool {
    PermissionSet::new(&[permission.to_owned()])
        .allows_any(contracts.iter().map(|name| name.as_str()))
}

/// The candidate closest to `name`, when it is a plausible typo of it.
pub(crate) fn closest<'a>(
    name: &str,
    candidates: impl Iterator<Item = &'a str>,
) -> Option<&'a str> {
    candidates
        .map(|candidate| (edit_distance(name, candidate), candidate))
        .filter(|(distance, _)| *distance <= 2)
        .min_by_key(|(distance, _)| *distance)
        .map(|(_, candidate)| candidate)
}

/// The number of single-character edits that turn `a` into `b`.
pub(crate) fn edit_distance(a: &str, b: &str) -> usize {
    let b: Vec<char> = b.chars().collect();
    let mut row: Vec<usize> = (0..=b.len()).collect();
    for (i, ca) in a.chars().enumerate() {
        let mut diagonal = row[0];
        row[0] = i + 1;
        for (j, &cb) in b.iter().enumerate() {
            let above = row[j + 1];
            row[j + 1] = if ca == cb {
                diagonal
            } else {
                1 + diagonal.min(above).min(row[j])
            };
            diagonal = above;
        }
    }
    row[b.len()]
}

/// `source`, the text of a `manivelle.kdl`, with `names` added to the `permissions` of its
/// package: after the permissions already there, which are kept with their comments, in a
/// `permissions` block that is created when there is none.
///
/// # Errors
///
/// Fails when `source` is not a manifest with a `package` node, or holds non-ASCII text,
/// which would make the positions the edit relies on unsafe.
pub fn add_permissions(source: &str, names: &[String]) -> Result<String, EngineError> {
    let fail = |details: &str| EngineError::Manifest {
        path: "manivelle.kdl".to_owned(),
        details: details.to_owned(),
    };
    if names.is_empty() {
        return Ok(source.to_owned());
    }
    if !source.is_ascii() {
        return Err(fail(
            "the manifest holds non-ASCII text; add the permissions by hand",
        ));
    }
    let document = KdlDocument::parse(source).map_err(|error| fail(&error.to_string()))?;
    let package = document
        .get("package")
        .ok_or_else(|| fail("no `package` node"))?;
    let package_range = range_of(package);
    let child = package
        .children()
        .and_then(|children| children.get("permissions"));

    match child {
        Some(node) if node.children().is_some() => {
            let range = range_of(node);
            let close = range.start
                + source[range.clone()]
                    .rfind('}')
                    .ok_or_else(|| fail("the `permissions` block has no closing brace"))?;
            Ok(insert_before_brace(source, close, names))
        }
        Some(node) => {
            // `permissions` with no block: replace it with one.
            let range = range_of(node);
            let indent = indent_of(source, range.start);
            let mut block = String::from("permissions {\n");
            for name in names {
                block.push_str(&format!("{indent}    {name}\n"));
            }
            block.push_str(&format!("{indent}}}"));
            Ok(format!(
                "{}{block}{}",
                &source[..range.start],
                &source[range.end..]
            ))
        }
        None => {
            let close = package_range.start
                + source[package_range.clone()]
                    .rfind('}')
                    .ok_or_else(|| fail("the `package` node has no block"))?;
            let indent = indent_of(source, package_range.start);
            let mut block = format!("{indent}    permissions {{\n");
            for name in names {
                block.push_str(&format!("{indent}        {name}\n"));
            }
            block.push_str(&format!("{indent}    }}\n"));
            Ok(insert_block_before_brace(source, close, &block))
        }
    }
}

/// The byte range of a node in an ASCII source.
fn range_of(node: &kdl::KdlNode) -> std::ops::Range<usize> {
    let span = node.span();
    span.offset()..span.offset() + span.len()
}

/// The whitespace a line starts with, for the line holding byte `at`.
fn indent_of(source: &str, at: usize) -> String {
    let line_start = source[..at].rfind('\n').map_or(0, |newline| newline + 1);
    source[line_start..]
        .chars()
        .take_while(|c| *c == ' ' || *c == '\t')
        .collect()
}

/// Adds `names` as lines inside the block that closes at `close`, one level deeper than
/// the line of the brace.
fn insert_before_brace(source: &str, close: usize, names: &[String]) -> String {
    let line_start = source[..close].rfind('\n').map_or(0, |newline| newline + 1);
    let indent = indent_of(source, close);
    let on_own_line = source[line_start..close].trim().is_empty();
    if on_own_line {
        let mut lines = String::new();
        for name in names {
            lines.push_str(&format!("{indent}    {name}\n"));
        }
        format!("{}{lines}{}", &source[..line_start], &source[line_start..])
    } else {
        let mut lines = String::new();
        for name in names {
            lines.push_str(&format!("\n{indent}    {name}"));
        }
        format!("{}{lines}\n{indent}{}", &source[..close], &source[close..])
    }
}

/// Adds `block` (complete lines) before the line that holds the brace at `close`.
fn insert_block_before_brace(source: &str, close: usize, block: &str) -> String {
    let line_start = source[..close].rfind('\n').map_or(0, |newline| newline + 1);
    if source[line_start..close].trim().is_empty() {
        format!("{}{block}{}", &source[..line_start], &source[line_start..])
    } else {
        // The brace shares its line with the package's last node: start a new line.
        format!("{}\n{block}{}", &source[..close], &source[close..])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn a_typo_gets_the_closest_candidate() {
        let candidates = ["world.modify", "world.read", "entity.spawn"];
        assert_eq!(
            closest("world.modfy", candidates.iter().copied()),
            Some("world.modify")
        );
        assert_eq!(closest("something.else", candidates.iter().copied()), None);
        assert_eq!(edit_distance("kitten", "sitting"), 3);
    }

    #[test]
    fn permissions_are_added_inside_an_existing_block_and_keep_its_comments() {
        let source = "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions {\n        // math.add\n        world.read\n    }\n}\n";
        let edited = add_permissions(source, &names(&["math.add", "world.modify"])).unwrap();

        assert_eq!(
            edited,
            "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions {\n        // math.add\n        world.read\n        math.add\n        world.modify\n    }\n}\n"
        );
        assert!(crate::PackageManifest::parse(&edited, std::path::Path::new("/p")).is_ok());
    }

    #[test]
    fn permissions_are_added_to_an_empty_or_missing_block() {
        let empty = "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions {\n        // none yet\n    }\n}\n";
        let edited = add_permissions(empty, &names(&["math.add"])).unwrap();
        assert!(
            edited.contains("        // none yet\n        math.add\n    }"),
            "{edited}"
        );

        let missing = "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n}\n";
        let edited = add_permissions(missing, &names(&["math.add", "events.*"])).unwrap();
        assert_eq!(
            edited,
            "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions {\n        math.add\n        events.*\n    }\n}\n"
        );
        let manifest = crate::PackageManifest::parse(&edited, std::path::Path::new("/p")).unwrap();
        assert_eq!(manifest.permissions, ["math.add", "events.*"]);
    }

    #[test]
    fn an_inline_block_is_opened_up() {
        let source = "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions { math.add }\n}\n";
        let edited = add_permissions(source, &names(&["world.modify"])).unwrap();
        let manifest = crate::PackageManifest::parse(&edited, std::path::Path::new("/p")).unwrap();
        assert_eq!(
            manifest.permissions,
            ["math.add", "world.modify"],
            "{edited}"
        );
    }

    #[test]
    fn non_ascii_manifests_are_left_alone() {
        let source = "package \"p\" {\n    version \"1.0.0\"\n    author \"Élodie\"\n    entry \"a.ts\"\n}\n";
        assert!(add_permissions(source, &names(&["math.add"])).is_err());
    }
}
