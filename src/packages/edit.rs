//! Editing the text of a `manivelle.kdl` to add permissions, keeping what is already there.

use kdl::KdlDocument;

use super::MANIFEST_FILE;
use crate::error::EngineError;

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
        path: MANIFEST_FILE.to_owned(),
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
    use crate::packages::PackageManifest;

    fn names(list: &[&str]) -> Vec<String> {
        list.iter().map(|name| (*name).to_owned()).collect()
    }

    #[test]
    fn permissions_are_added_inside_an_existing_block_and_keep_its_comments() {
        let source = "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions {\n        // math.add\n        world.read\n    }\n}\n";
        let edited = add_permissions(source, &names(&["math.add", "world.modify"])).unwrap();

        assert_eq!(
            edited,
            "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions {\n        // math.add\n        world.read\n        math.add\n        world.modify\n    }\n}\n"
        );
        assert!(PackageManifest::parse(&edited, std::path::Path::new("/p")).is_ok());
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
        let manifest = PackageManifest::parse(&edited, std::path::Path::new("/p")).unwrap();
        assert_eq!(manifest.permissions, ["math.add", "events.*"]);
    }

    #[test]
    fn an_inline_block_is_opened_up() {
        let source = "package \"p\" {\n    version \"1.0.0\"\n    entry \"a.ts\"\n    permissions { math.add }\n}\n";
        let edited = add_permissions(source, &names(&["world.modify"])).unwrap();
        let manifest = PackageManifest::parse(&edited, std::path::Path::new("/p")).unwrap();
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
