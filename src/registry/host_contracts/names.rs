//! Identifier rules shared by the declarations, the SDK and the import modules.

use std::collections::{BTreeMap, BTreeSet};

/// ECMAScript reserved words and literals, which cannot name a binding.
const RESERVED_WORDS: &[&str] = &[
    "await",
    "break",
    "case",
    "catch",
    "class",
    "const",
    "continue",
    "debugger",
    "default",
    "delete",
    "do",
    "else",
    "enum",
    "export",
    "extends",
    "false",
    "finally",
    "for",
    "function",
    "if",
    "import",
    "in",
    "instanceof",
    "let",
    "new",
    "null",
    "return",
    "static",
    "super",
    "switch",
    "this",
    "throw",
    "true",
    "try",
    "typeof",
    "var",
    "void",
    "while",
    "with",
    "yield",
];

/// Whether `name` is an ASCII JavaScript identifier name, reserved words included, which
/// are legal as a property key.
pub(super) fn is_identifier_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };

    (first == '_' || first == '$' || first.is_ascii_alphabetic())
        && chars.all(|ch| ch == '_' || ch == '$' || ch.is_ascii_alphanumeric())
}

/// Whether `name` can name a binding: an identifier name that is not reserved.
pub(super) fn is_binding_name(name: &str) -> bool {
    is_identifier_name(name) && !RESERVED_WORDS.contains(&name)
}

/// `name` as a property key of an object literal: bare when it is a binding name, a
/// string literal otherwise.
pub(super) fn property_name(name: &str) -> String {
    if is_binding_name(name) {
        name.to_owned()
    } else {
        format!("{name:?}")
    }
}

/// The binding name of each name of one level of exports: the name itself when it is a
/// binding name, else a fresh `sdk`, `sdk2`, ... that no other name of the level takes.
pub(super) fn binding_names<'a>(
    names: impl IntoIterator<Item = &'a str>,
) -> BTreeMap<String, String> {
    let names: BTreeSet<&str> = names.into_iter().collect();
    let mut taken: BTreeSet<String> = names
        .iter()
        .filter(|name| is_binding_name(name))
        .map(|name| (*name).to_owned())
        .collect();

    names
        .into_iter()
        .map(|name| {
            if is_binding_name(name) {
                return (name.to_owned(), name.to_owned());
            }
            let mut candidate = String::from("sdk");
            let mut suffix = 1;
            while !taken.insert(candidate.clone()) {
                suffix += 1;
                candidate = format!("sdk{suffix}");
            }
            (name.to_owned(), candidate)
        })
        .collect()
}

/// `depth` levels of two-space indentation.
pub(super) fn indent(depth: usize) -> String {
    "  ".repeat(depth)
}
