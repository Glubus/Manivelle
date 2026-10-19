use std::collections::BTreeMap;

use crate::contract::{HostContractAbi, HostContractDescriptor};

use super::names::{binding_names, indent, property_name};
use super::tree::{ObjectNode, ObjectTree};

#[derive(Debug, Clone)]
enum HostModuleBinding {
    Function { contract_name: String },
    Callback { event_name: String },
}

/// Where a host import module reads its native functions and its handler registration
/// from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum HostModuleStyle {
    /// The globals of the script's own context: `globalThis.__manivelle_native` and
    /// `globalThis.__manivelle_on`.
    Globals,
    /// The script's environment, imported from `manivelle:env`: the module of a script in a
    /// context group, whose context has no per-script globals.
    Env,
}

/// Source of every host import module. Function exports are the native functions the
/// engine binds to the importing script; callback exports register handlers.
pub(crate) fn render_host_import_modules(
    descriptors: &[HostContractDescriptor],
    style: HostModuleStyle,
) -> BTreeMap<String, String> {
    let mut modules = BTreeMap::<String, ObjectTree<HostModuleBinding>>::new();

    for descriptor in descriptors {
        let Some(binding) = binding_for_descriptor(descriptor) else {
            continue;
        };
        let module = modules.entry(descriptor.import.module.clone()).or_default();
        let path = &descriptor.import.export_path;
        if path.is_empty() {
            // The default `EXPORT_PATH`: the contract is reached through the SDK only and
            // has no name in the import module.
            continue;
        }
        module.insert_segments(path.iter().map(String::as_str), binding);
    }

    modules
        .into_iter()
        .map(|(module, tree)| (module, render_module_source(&tree, style)))
        .collect()
}

/// Runtime binding exported for one contract. Contexts are declarative only: nothing
/// provides their value at runtime, so importing one fails at load instead of
/// silently binding `undefined`.
fn binding_for_descriptor(descriptor: &HostContractDescriptor) -> Option<HostModuleBinding> {
    match &descriptor.abi {
        HostContractAbi::Function { .. } => Some(HostModuleBinding::Function {
            contract_name: descriptor.name.clone(),
        }),
        HostContractAbi::Callback { .. } => Some(HostModuleBinding::Callback {
            event_name: descriptor.name.clone(),
        }),
        HostContractAbi::Context { .. }
        | HostContractAbi::Export { .. }
        | HostContractAbi::Unknown => None,
    }
}

fn render_module_source(tree: &ObjectTree<HostModuleBinding>, style: HostModuleStyle) -> String {
    let mut output = String::new();
    if style == HostModuleStyle::Env {
        output.push_str("import { __native, __on } from \"manivelle:env\";\n");
    }

    let names = binding_names(tree.roots.keys().map(String::as_str));
    for (name, node) in &tree.roots {
        output.push_str("export const ");
        output.push_str(&names[name]);
        output.push_str(" = ");
        render_node(node, 0, style, &mut output);
        output.push_str(";\n");
    }

    output
}

fn render_node(
    node: &ObjectNode<HostModuleBinding>,
    depth: usize,
    style: HostModuleStyle,
    output: &mut String,
) {
    if let Some(binding) = &node.binding
        && node.children.is_empty()
    {
        render_binding(binding, style, output);
        return;
    }

    render_object_node(node, depth, style, output);
}

fn render_object_node(
    node: &ObjectNode<HostModuleBinding>,
    depth: usize,
    style: HostModuleStyle,
    output: &mut String,
) {
    if node.children.is_empty() && node.binding.is_none() {
        output.push_str("{}");
        return;
    }

    let inner = indent(depth + 1);
    output.push_str("{\n");
    let mut separator = "";
    for (name, child) in &node.children {
        output.push_str(separator);
        output.push_str(&inner);
        output.push_str(&property_name(name));
        output.push_str(": ");
        render_node(child, depth + 1, style, output);
        separator = ",\n";
    }

    if let Some(binding) = &node.binding {
        output.push_str(separator);
        output.push_str(&inner);
        output.push_str("default: ");
        render_binding(binding, style, output);
    }

    output.push('\n');
    output.push_str(&indent(depth));
    output.push('}');
}

fn render_binding(binding: &HostModuleBinding, style: HostModuleStyle, output: &mut String) {
    let source = match (binding, style) {
        (HostModuleBinding::Function { contract_name }, HostModuleStyle::Globals) => {
            format!("globalThis.__manivelle_native[{contract_name:?}]")
        }
        (HostModuleBinding::Function { contract_name }, HostModuleStyle::Env) => {
            format!("__native[{contract_name:?}]")
        }
        (HostModuleBinding::Callback { event_name }, HostModuleStyle::Globals) => {
            format!("handler => globalThis.__manivelle_on({event_name:?}, handler)")
        }
        (HostModuleBinding::Callback { event_name }, HostModuleStyle::Env) => {
            format!("handler => __on({event_name:?}, handler)")
        }
    };
    output.push_str(&source);
}
