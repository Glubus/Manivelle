//! What a package may reach on the target.

use std::collections::HashSet;
use std::path::{Path, PathBuf};

/// The host contracts a package declared in its `permissions` block.
///
/// A permission is the name of a contract (`world.modify`), or a namespace with a
/// wildcard (`world.*`: every contract whose name starts with `world.`), or `*` for
/// all of them. A contract is the name of a host function or of an event.
///
/// The set also carries the canonical root directory of the package it restricts, when
/// there is one: the package's modules may not import from outside it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub(crate) struct PermissionSet {
    exact: HashSet<String>,
    prefixes: Vec<String>,
    all: bool,
    package_root: Option<PathBuf>,
}

impl PermissionSet {
    /// The set the declared permissions make; none declared allows nothing.
    pub(crate) fn new(declared: &[String]) -> Self {
        let mut set = Self::default();
        for permission in declared {
            if permission == "*" {
                set.all = true;
            } else if let Some(prefix) = permission.strip_suffix('*')
                && prefix.ends_with('.')
            {
                set.prefixes.push(prefix.to_owned());
            } else {
                set.exact.insert(permission.clone());
            }
        }
        set
    }

    /// This set, restricting the package whose canonical root directory is `root`.
    pub(crate) fn with_package_root(mut self, root: PathBuf) -> Self {
        self.package_root = Some(root);
        self
    }

    /// The canonical root directory of the package this set restricts.
    pub(crate) fn package_root(&self) -> Option<&Path> {
        self.package_root.as_deref()
    }

    /// Whether the package may use contract `contract`.
    pub(crate) fn allows(&self, contract: &str) -> bool {
        self.all
            || self.exact.contains(contract)
            || self
                .prefixes
                .iter()
                .any(|prefix| contract.starts_with(prefix))
    }

    /// Whether the package may use at least one of `contracts`.
    pub(crate) fn allows_any<'a>(&self, mut contracts: impl Iterator<Item = &'a str>) -> bool {
        contracts.any(|contract| self.allows(contract))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn set(declared: &[&str]) -> PermissionSet {
        PermissionSet::new(&declared.iter().map(|p| (*p).to_owned()).collect::<Vec<_>>())
    }

    #[test]
    fn a_permission_is_an_exact_contract_name() {
        let set = set(&["world.modify"]);
        assert!(set.allows("world.modify"));
        assert!(!set.allows("world.read"));
        assert!(!set.allows("world.modify.all"));
        assert!(!set.allows("entity.spawn"));
    }

    #[test]
    fn a_trailing_star_allows_a_namespace_and_a_lone_star_everything() {
        let namespace = set(&["events.*"]);
        assert!(namespace.allows("events.listen"));
        assert!(namespace.allows("events.a.b"));
        assert!(!namespace.allows("events"));
        assert!(!namespace.allows("eventsx.listen"));

        assert!(set(&["*"]).allows("anything.at.all"));
    }

    #[test]
    fn a_star_that_does_not_follow_a_dot_is_no_wildcard() {
        let set = set(&["events*"]);
        assert!(!set.allows("eventsx.listen"));
        assert!(!set.allows("events.listen"));
        assert!(!set.allows("events"));
    }

    #[test]
    fn nothing_declared_allows_nothing() {
        assert!(!set(&[]).allows("world.modify"));
    }
}
