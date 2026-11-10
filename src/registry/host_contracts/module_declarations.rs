//! The `declare module "<IMPORT_MODULE>"` blocks of the declarations: what a script gets
//! from `import { game } from "game"`, typed the way the engine's virtual module of that
//! name exports it (see `import_modules.rs`, which builds the same tree for the runtime).

use std::collections::BTreeMap;

use crate::contract::{HostContractAbi, HostContractDescriptor};

use super::declarations::{function_output_type, render_doc_comment, schema_type_name};
use super::names::{binding_names, indent, property_name};
use super::tree::{ObjectNode, ObjectTree};

/// One export of an import module, with its types already rendered.
pub(super) enum ModuleExport {
    /// A host function, called with its input.
    Function {
        input: String,
        output: String,
        docs: Option<String>,
    },
    /// A callback, exported as the function that registers a handler for `event`.
    Callback { event: String, docs: Option<String> },
}

impl ModuleExport {
    /// `(parameter list, return type)` of the exported function.
    fn signature(&self) -> (String, String) {
        match self {
            Self::Function { input, output, .. } => (format!("input: {input}"), output.clone()),
            Self::Callback { event, .. } => (
                format!("handler: HostEventHandler<{event:?}>"),
                "void".to_owned(),
            ),
        }
    }

    fn docs(&self) -> Option<&str> {
        match self {
            Self::Function { docs, .. } | Self::Callback { docs, .. } => docs.as_deref(),
        }
    }
}

/// The exports of every import module, by module name.
#[derive(Default)]
pub(super) struct ImportModules(BTreeMap<String, ObjectTree<ModuleExport>>);

impl ImportModules {
    /// Adds the contract at its `EXPORT_PATH` in its `IMPORT_MODULE`. Contracts that are
    /// not importable (no module, no path) and kinds the virtual module does not export
    /// (contexts, script exports) add nothing. The schemas of the contract must have been
    /// emitted already, so that their names are declared.
    pub(super) fn push(&mut self, descriptor: &HostContractDescriptor) {
        let import = &descriptor.import;
        if import.module.is_empty() || import.export_path.is_empty() {
            return;
        }
        let docs = descriptor.docs.clone();
        let export = match &descriptor.abi {
            HostContractAbi::Function {
                input,
                output,
                returns_promise,
            } => ModuleExport::Function {
                input: schema_type_name(input),
                output: function_output_type(output, *returns_promise),
                docs,
            },
            HostContractAbi::Callback { .. } => ModuleExport::Callback {
                event: descriptor.name.clone(),
                docs,
            },
            HostContractAbi::Context { .. }
            | HostContractAbi::Export { .. }
            | HostContractAbi::Unknown => return,
        };
        self.0
            .entry(import.module.clone())
            .or_default()
            .insert_segments(import.export_path.iter().map(String::as_str), export);
    }

    /// One `declare module` block per module, in module-name order.
    pub(super) fn render(&self) -> Vec<String> {
        self.0
            .iter()
            .filter(|(_, tree)| !tree.is_empty())
            .map(|(module, tree)| render_module(module, tree))
            .collect()
    }
}

fn render_module(module: &str, tree: &ObjectTree<ModuleExport>) -> String {
    // The runtime module exports each root under this name too.
    let names = binding_names(tree.roots.keys().map(String::as_str), &[]);
    let exports: Vec<String> = tree
        .roots
        .iter()
        .map(|(name, node)| render_root(&names[name], node))
        .collect();
    format!("declare module {module:?} {{\n{}\n}}", exports.join("\n"))
}

/// One export of the module: a function when the path ends there, else an object.
fn render_root(name: &str, node: &ObjectNode<ModuleExport>) -> String {
    let pad = indent(1);
    match (&node.binding, node.children.is_empty()) {
        (Some(export), true) => {
            let (parameters, returns) = export.signature();
            format!(
                "{}{pad}export function {name}({parameters}): {returns};",
                docs_of(export, &pad)
            )
        }
        _ => format!(
            "{pad}export const {name}: {};",
            render_object(node, 1).trim_start()
        ),
    }
}

/// `{ ... }` holding the members of `node`, closed at `depth`. A node that is also a
/// function exports it as `default`, as the runtime module does.
fn render_object(node: &ObjectNode<ModuleExport>, depth: usize) -> String {
    let pad = indent(depth);
    let mut output = format!("{pad}{{\n");
    for (name, child) in &node.children {
        output.push_str(&render_member(name, child, depth + 1));
    }
    if let Some(export) = &node.binding {
        output.push_str(&render_method("default", export, depth + 1));
    }
    output.push_str(&pad);
    output.push('}');
    output
}

fn render_member(name: &str, node: &ObjectNode<ModuleExport>, depth: usize) -> String {
    match (&node.binding, node.children.is_empty()) {
        (Some(export), true) => render_method(&property_name(name), export, depth),
        _ => format!(
            "{}{}: {};\n",
            indent(depth),
            property_name(name),
            render_object(node, depth).trim_start()
        ),
    }
}

fn render_method(key: &str, export: &ModuleExport, depth: usize) -> String {
    let pad = indent(depth);
    let (parameters, returns) = export.signature();
    format!(
        "{}{pad}{key}({parameters}): {returns};\n",
        docs_of(export, &pad)
    )
}

fn docs_of(export: &ModuleExport, pad: &str) -> String {
    export
        .docs()
        .map(|docs| render_doc_comment(docs, pad))
        .unwrap_or_default()
}
