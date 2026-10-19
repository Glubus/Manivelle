//! The `manivelle.kdl` manifest of a package.
//!
//! ```kdl
//! package "better-combat" {
//!     version "1.2.0"
//!     author "dev"
//!
//!     engine {
//!         requires ">=0.1.0"
//!     }
//!
//!     depends {
//!         "core" version "^1.0"
//!     }
//!
//!     permissions {
//!         world.modify
//!         entity.spawn
//!     }
//!
//!     entry "src/index.ts"
//!     target "../game"
//! }
//! ```
//!
//! This is [KDL 2](https://kdl.dev): a node is a name, arguments (`"quoted"` or bare
//! words) and an optional `{ ... }` block; `//`, `/* */` and `/-` comment. The manifest
//! only uses string values and no `key=value` properties, and unknown nodes are errors,
//! so a misspelled key cannot silently do nothing.

use std::fs;
use std::path::{Component, Path, PathBuf};

use kdl::{KdlDocument, KdlError};
use semver::{Version, VersionReq};

use crate::compiler::LineIndex;
use crate::error::EngineError;

/// The file name of a package manifest.
pub const MANIFEST_FILE: &str = "manivelle.kdl";

/// A package this package needs, and the versions of it that work.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageDependency {
    /// The name of the required package.
    pub name: String,
    /// The accepted versions; any version when the manifest gives none.
    pub requirement: VersionReq,
}

/// A parsed `manivelle.kdl` manifest.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct PackageManifest {
    /// The package's name, also the id of its script in the engine.
    pub name: String,
    /// The package's version.
    pub version: Version,
    /// Who wrote it.
    pub author: Option<String>,
    /// The versions of Manivelle the package works with; any when absent.
    pub engine_requires: Option<VersionReq>,
    /// The packages loaded before this one.
    pub depends: Vec<PackageDependency>,
    /// What the package may use on the target: host contract names, `namespace.*` or `*`.
    /// A package that lists none reaches no host function or event.
    pub permissions: Vec<String>,
    /// The entry file, relative to [`root`](Self::root); it cannot leave it.
    pub entry: PathBuf,
    /// The program `manivelle run` starts with this package, as written in the manifest.
    pub target: Option<PathBuf>,
    /// The directory of the manifest.
    pub root: PathBuf,
}

impl PackageManifest {
    /// Reads the manifest of the package in directory `root`.
    ///
    /// # Errors
    ///
    /// Fails when the file cannot be read or is not a valid manifest.
    pub fn read(root: &Path) -> Result<Self, EngineError> {
        let path = root.join(MANIFEST_FILE);
        let source = fs::read_to_string(&path).map_err(|error| EngineError::Manifest {
            path: path.display().to_string(),
            details: error.to_string(),
        })?;
        Self::parse(&source, root)
    }

    /// Parses manifest `source` of the package in directory `root`.
    ///
    /// # Errors
    ///
    /// Fails, with the line, on text that is not KDL, a value that is not a string, an
    /// unknown node, a missing `version` or `entry`, a version that is not semver, or an
    /// `entry` that is absolute or leaves the package's directory.
    pub fn parse(source: &str, root: &Path) -> Result<Self, EngineError> {
        let fail = |failure: Failure| EngineError::Manifest {
            path: root.join(MANIFEST_FILE).display().to_string(),
            details: failure.to_string(),
        };
        let nodes = parse_tree(source).map_err(fail)?;
        Self::from_nodes(&nodes, root).map_err(fail)
    }

    /// The path of the entry file.
    #[must_use]
    pub fn entry_path(&self) -> PathBuf {
        self.root.join(&self.entry)
    }

    fn from_nodes(nodes: &[Node], root: &Path) -> Result<Self, Failure> {
        let [node] = nodes else {
            return Err(Failure::at(
                nodes.get(1).map_or(1, |extra| extra.line),
                "expected one `package \"name\" { ... }` block",
            ));
        };
        if node.name != "package" {
            return Err(Failure::at(
                node.line,
                "expected `package \"name\" { ... }`",
            ));
        }
        let name = node.one_arg()?.to_owned();
        check_name(&name, node.line)?;
        let mut fields = Fields::default();
        for child in &node.children {
            fields.take(child)?;
        }
        let version = fields
            .version
            .ok_or_else(|| Failure::at(node.line, "missing `version`"))?;
        let entry = fields
            .entry
            .ok_or_else(|| Failure::at(node.line, "missing `entry`"))?;
        Ok(Self {
            name,
            version,
            author: fields.author,
            engine_requires: fields.engine_requires,
            depends: fields.depends,
            permissions: fields.permissions,
            entry,
            target: fields.target,
            root: root.to_path_buf(),
        })
    }
}

/// The nodes of a `package` block, as they are read.
#[derive(Default)]
struct Fields {
    version: Option<Version>,
    author: Option<String>,
    engine_requires: Option<VersionReq>,
    depends: Vec<PackageDependency>,
    permissions: Vec<String>,
    entry: Option<PathBuf>,
    target: Option<PathBuf>,
    seen: Vec<String>,
}

impl Fields {
    fn take(&mut self, node: &Node) -> Result<(), Failure> {
        if self.seen.contains(&node.name) {
            return Err(Failure::at(
                node.line,
                format!("`{}` appears twice", node.name),
            ));
        }
        self.seen.push(node.name.clone());
        match node.name.as_str() {
            "version" => self.version = Some(parse_version(node.one_arg()?, node.line)?),
            "author" => self.author = Some(node.one_arg()?.to_owned()),
            "entry" => self.entry = Some(contained_path(node.one_arg()?, node.line)?),
            "target" => self.target = Some(PathBuf::from(node.one_arg()?)),
            "engine" => self.engine_requires = Some(engine_requirement(node)?),
            "depends" => self.depends = dependencies(node)?,
            "permissions" => self.permissions = permissions(node)?,
            other => {
                return Err(Failure::at(
                    node.line,
                    format!(
                        "unknown `{other}` (expected version, author, engine, depends, \
                         permissions, entry or target)"
                    ),
                ));
            }
        }
        Ok(())
    }
}

fn engine_requirement(node: &Node) -> Result<VersionReq, Failure> {
    node.no_args()?;
    let [requires] = node.children.as_slice() else {
        return Err(Failure::at(
            node.line,
            "`engine` holds one `requires \"...\"`",
        ));
    };
    if requires.name != "requires" {
        return Err(Failure::at(requires.line, "expected `requires \"...\"`"));
    }
    parse_requirement(requires.one_arg()?, requires.line)
}

fn dependencies(node: &Node) -> Result<Vec<PackageDependency>, Failure> {
    node.no_args()?;
    let mut found: Vec<PackageDependency> = Vec::new();
    for child in &node.children {
        check_name(&child.name, child.line)?;
        if found.iter().any(|known| known.name == child.name) {
            return Err(Failure::at(
                child.line,
                format!("`{}` is listed twice", child.name),
            ));
        }
        child.no_children()?;
        let requirement = match child.args.as_slice() {
            [] => VersionReq::STAR,
            [keyword, requirement] if keyword == "version" => {
                parse_requirement(requirement, child.line)?
            }
            _ => {
                return Err(Failure::at(
                    child.line,
                    "expected `\"name\"` or `\"name\" version \"^1.0\"`",
                ));
            }
        };
        found.push(PackageDependency {
            name: child.name.clone(),
            requirement,
        });
    }
    Ok(found)
}

fn permissions(node: &Node) -> Result<Vec<String>, Failure> {
    node.no_args()?;
    let mut found: Vec<String> = Vec::new();
    for child in &node.children {
        child.no_args()?;
        child.no_children()?;
        let valid = well_formed(&child.name, &['_', '.', '*', '-']);
        if !valid {
            return Err(Failure::at(
                child.line,
                format!("`{}` is not a permission name", child.name),
            ));
        }
        if !found.contains(&child.name) {
            found.push(child.name.clone());
        }
    }
    Ok(found)
}

/// Whether `name` is not empty and has only ASCII letters and digits, and the `extra`
/// characters.
fn well_formed(name: &str, extra: &[char]) -> bool {
    !name.is_empty()
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || extra.contains(&c))
}

fn check_name(name: &str, line: usize) -> Result<(), Failure> {
    if well_formed(name, &['_', '.', '-']) {
        Ok(())
    } else {
        Err(Failure::at(
            line,
            format!("`{name}` is not a package name (letters, digits, `_`, `-` and `.`)"),
        ))
    }
}

fn parse_version(text: &str, line: usize) -> Result<Version, Failure> {
    Version::parse(text)
        .map_err(|error| Failure::at(line, format!("`{text}` is not a version: {error}")))
}

fn parse_requirement(text: &str, line: usize) -> Result<VersionReq, Failure> {
    VersionReq::parse(text).map_err(|error| {
        Failure::at(
            line,
            format!("`{text}` is not a version requirement: {error}"),
        )
    })
}

/// `text` as a path that stays inside the package's directory.
fn contained_path(text: &str, line: usize) -> Result<PathBuf, Failure> {
    let path = PathBuf::from(text);
    let inside = !text.is_empty()
        && path
            .components()
            .all(|part| matches!(part, Component::Normal(_) | Component::CurDir));
    if inside {
        Ok(path)
    } else {
        Err(Failure::at(
            line,
            format!("`{text}` must be a relative path inside the package's directory"),
        ))
    }
}

/// A syntax or meaning error at a line of the manifest.
#[derive(Debug)]
struct Failure {
    line: usize,
    message: String,
}

impl Failure {
    fn at(line: usize, message: impl Into<String>) -> Self {
        Self {
            line,
            message: message.into(),
        }
    }
}

impl std::fmt::Display for Failure {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(formatter, "line {}: {}", self.line, self.message)
    }
}

/// One node of the manifest: a name, its string arguments and its block.
struct Node {
    name: String,
    args: Vec<String>,
    children: Vec<Node>,
    line: usize,
}

impl Node {
    fn one_arg(&self) -> Result<&str, Failure> {
        match self.args.as_slice() {
            [arg] => Ok(arg),
            _ => Err(Failure::at(
                self.line,
                format!("`{}` takes one value", self.name),
            )),
        }
    }

    fn no_args(&self) -> Result<(), Failure> {
        if self.args.is_empty() {
            Ok(())
        } else {
            Err(Failure::at(
                self.line,
                format!("`{}` takes no value here", self.name),
            ))
        }
    }

    fn no_children(&self) -> Result<(), Failure> {
        if self.children.is_empty() {
            Ok(())
        } else {
            Err(Failure::at(
                self.line,
                format!("`{}` takes no block here", self.name),
            ))
        }
    }
}

/// The nodes of `source`, read as KDL 2.
fn parse_tree(source: &str) -> Result<Vec<Node>, Failure> {
    let lines = LineIndex::new(source);
    let document = KdlDocument::parse(source).map_err(|error| syntax_failure(&lines, &error))?;
    nodes_of(&lines, &document)
}

fn nodes_of(lines: &LineIndex<'_>, document: &KdlDocument) -> Result<Vec<Node>, Failure> {
    document
        .nodes()
        .iter()
        .map(|node| {
            let line = lines.line_of(node.span().offset());
            let name = node.name().value().to_owned();
            let mut args = Vec::new();
            for entry in node.entries() {
                if entry.name().is_some() {
                    return Err(Failure::at(
                        line,
                        format!("`{name}` takes values, not `key=value` properties"),
                    ));
                }
                let Some(text) = entry.value().as_string() else {
                    return Err(Failure::at(
                        line,
                        format!("`{name}` takes strings: quote versions and paths"),
                    ));
                };
                args.push(text.to_owned());
            }
            let children = match node.children() {
                Some(block) => nodes_of(lines, block)?,
                None => Vec::new(),
            };
            Ok(Node {
                name,
                args,
                children,
                line,
            })
        })
        .collect()
}

fn syntax_failure(lines: &LineIndex<'_>, error: &KdlError) -> Failure {
    let Some(first) = error.diagnostics.first() else {
        return Failure::at(1, error.to_string());
    };
    let mut message = first
        .message
        .clone()
        .unwrap_or_else(|| "not valid KDL".to_owned());
    if let Some(help) = &first.help {
        message.push_str(&format!(" ({help})"));
    }
    Failure::at(lines.line_of(first.span.offset()), message)
}

#[cfg(test)]
mod tests {
    use super::*;

    const FULL: &str = r#"
// a combat package
package "better-combat" {
    version "1.2.0"
    author "dev"

    engine {
        requires ">=0.1.0"
    }

    depends {
        "core" version "^1.0"
        "ui"
    }

    permissions {
        world.modify   // may change the world
        entity.spawn
        events.*
    }

    /* the entry */
    entry "src/index.ts"
    target "../game"
}
"#;

    fn parse(source: &str) -> Result<PackageManifest, String> {
        PackageManifest::parse(source, Path::new("/packages/better-combat"))
            .map_err(|e| e.to_string())
    }

    #[test]
    fn reads_every_field() {
        let manifest = parse(FULL).unwrap();

        assert_eq!(manifest.name, "better-combat");
        assert_eq!(manifest.version, Version::new(1, 2, 0));
        assert_eq!(manifest.author.as_deref(), Some("dev"));
        assert_eq!(
            manifest.engine_requires,
            Some(VersionReq::parse(">=0.1.0").unwrap())
        );
        assert_eq!(manifest.depends.len(), 2);
        assert_eq!(manifest.depends[0].name, "core");
        assert_eq!(
            manifest.depends[0].requirement,
            VersionReq::parse("^1.0").unwrap()
        );
        assert_eq!(manifest.depends[1].requirement, VersionReq::STAR);
        assert_eq!(
            manifest.permissions,
            ["world.modify", "entity.spawn", "events.*"]
        );
        assert_eq!(
            manifest.entry_path(),
            Path::new("/packages/better-combat/src/index.ts")
        );
        assert_eq!(manifest.target, Some(PathBuf::from("../game")));
    }

    #[test]
    fn a_minimal_manifest_needs_a_version_and_an_entry() {
        let minimal = "package \"tiny\" {\n version \"0.1.0\"\n entry \"main.ts\"\n}\n";
        let manifest = parse(minimal).unwrap();
        assert!(manifest.depends.is_empty() && manifest.permissions.is_empty());

        assert!(
            parse("package \"tiny\" { entry \"main.ts\" }")
                .unwrap_err()
                .contains("missing `version`")
        );
        assert!(
            parse("package \"tiny\" {\n version \"0.1.0\"\n}")
                .unwrap_err()
                .contains("missing `entry`")
        );
    }

    #[test]
    fn a_misspelled_node_is_an_error_with_its_line() {
        let error = parse("package \"m\" {\n version \"1.0.0\"\n entery \"a.ts\"\n}").unwrap_err();
        assert!(
            error.contains("line 3") && error.contains("unknown `entery`"),
            "{error}"
        );
    }

    #[test]
    fn error_lines_count_bytes_before_them_not_characters() {
        // Each `é` is two bytes: the unknown node on line 2 is still on line 2.
        let error = parse("package \"m\" { // ééééé\n  b\n}").unwrap_err();
        assert!(
            error.contains("line 2") && error.contains("unknown `b`"),
            "{error}"
        );
    }

    #[test]
    fn rejects_entries_that_leave_the_package_directory() {
        for entry in ["../x.ts", "/etc/x.ts", "a/../../x.ts"] {
            let source = format!("package \"m\" {{\n version \"1.0.0\"\n entry \"{entry}\"\n}}");
            assert!(
                parse(&source)
                    .unwrap_err()
                    .contains("inside the package's directory"),
                "{entry}"
            );
        }
    }

    #[test]
    fn rejects_bad_versions_names_and_values() {
        let wrap = |body: &str| format!("package \"m\" {{\n entry \"a.ts\"\n {body}\n}}");
        assert!(
            parse(&wrap("version \"one\""))
                .unwrap_err()
                .contains("is not a version")
        );
        assert!(
            parse("package \"a b/c\" {\n}")
                .unwrap_err()
                .contains("is not a package name")
        );
        assert!(
            parse(&wrap("version \"1.0.0\"\n version \"1.0.1\""))
                .unwrap_err()
                .contains("twice")
        );
        assert!(parse("").unwrap_err().contains("expected one"));
        // A number is not a string: versions are quoted.
        assert!(
            parse(&wrap("version 1.5"))
                .unwrap_err()
                .contains("takes strings")
        );
        assert!(
            parse(&wrap("author name=\"x\""))
                .unwrap_err()
                .contains("properties")
        );
    }

    #[test]
    fn invalid_kdl_is_reported_at_its_line() {
        for broken in [
            "package \"m\" {\n version \"1.0.0\"",
            "package \"m\" {\n author \"x\n}",
            "}",
        ] {
            let error = parse(broken).unwrap_err();
            assert!(error.contains("line "), "{broken:?}: {error}");
        }
        let error = parse("package \"m\" {\n version \"1.0.0\"\n author \"x\n}").unwrap_err();
        assert!(error.contains("line 3"), "{error}");
    }
}
