use std::collections::BTreeMap;

use crate::contract::TsType;

use super::super::declarations::render_doc_comment;
use super::super::names::{indent, property_name};
use super::super::tree::{ObjectNode, ObjectTree};
use super::builder::{
    SdkContext, SdkDomainBinding, SdkEvent, SdkEventAlias, SdkFunction, SdkFunctionType,
};

/// The merged tree of the domain exports: function bindings and event aliases.
pub(super) fn domain_tree(
    functions: &ObjectTree<SdkFunction>,
    events: &ObjectTree<SdkEvent>,
) -> ObjectTree<SdkDomainBinding> {
    let mut tree = ObjectTree::default();
    push_function_domain_bindings(&mut tree, &functions.roots, "");
    push_event_alias_domain_bindings(&mut tree, events);
    tree
}

pub(super) fn render_domain_exports(
    tree: &ObjectTree<SdkDomainBinding>,
    names: &BTreeMap<String, String>,
) -> Vec<String> {
    render_exported_tree(tree, names, render_domain_binding)
}

fn push_function_domain_bindings(
    output: &mut ObjectTree<SdkDomainBinding>,
    nodes: &BTreeMap<String, ObjectNode<SdkFunction>>,
    prefix: &str,
) {
    for (name, node) in nodes {
        let path = child_path(prefix, name);
        if let Some(function) = &node.binding {
            output.insert(&path, SdkDomainBinding::Function(function.clone()));
        }
        push_function_domain_bindings(output, &node.children, &path);
    }
}

fn push_event_alias_domain_bindings(
    output: &mut ObjectTree<SdkDomainBinding>,
    events: &ObjectTree<SdkEvent>,
) {
    for (root, node) in &events.roots {
        push_event_aliases_for_root(output, root, node, &mut Vec::new());
    }
}

/// Adds the alias of every event under `node`; `segments` is the path from the root to
/// `node`, extended in place for each child and restored after it.
fn push_event_aliases_for_root(
    output: &mut ObjectTree<SdkDomainBinding>,
    root: &str,
    node: &ObjectNode<SdkEvent>,
    segments: &mut Vec<String>,
) {
    if let Some(event) = &node.binding
        && let Some(alias) = event_alias_path(root, segments)
    {
        output.insert(
            &alias,
            SdkDomainBinding::EventAlias(SdkEventAlias {
                event_name: event.event_name.clone(),
            }),
        );
    }

    for (name, child) in &node.children {
        segments.push(name.clone());
        push_event_aliases_for_root(output, root, child, segments);
        segments.pop();
    }
}

fn event_alias_path(root: &str, segments: &[String]) -> Option<String> {
    if segments.is_empty() {
        return None;
    }

    Some(format!("{root}.on{}", pascal_case(segments)))
}

fn child_path(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_owned()
    } else {
        format!("{prefix}.{name}")
    }
}

fn pascal_case(segments: &[String]) -> String {
    segments
        .iter()
        .map(|segment| {
            segment
                .split(['-', '_', ' '])
                .filter(|part| !part.is_empty())
                .map(capitalize_identifier_part)
                .collect::<String>()
        })
        .collect::<String>()
}

fn capitalize_identifier_part(part: &str) -> String {
    let mut chars = part.chars();
    let Some(first) = chars.next() else {
        return String::new();
    };

    format!("{}{}", first.to_ascii_uppercase(), chars.as_str())
}

pub(super) fn render_context_exports(
    tree: &ObjectTree<SdkContext>,
    names: &BTreeMap<String, String>,
) -> Vec<String> {
    tree.roots
        .iter()
        .filter_map(|(name, node)| render_context_export(&names[name], node))
        .collect()
}

fn render_context_export(binding: &str, node: &ObjectNode<SdkContext>) -> Option<String> {
    node.binding.as_ref().map(|context| {
        format!(
            "export const {binding} = (globalThis as unknown as Record<string, unknown>)[{:?}] as {};",
            context.contract_name,
            context.ty
        )
    })
}

pub(super) fn render_events_export(tree: &ObjectTree<SdkEvent>) -> Option<String> {
    if tree.is_empty() {
        return None;
    }

    Some(render_named_object_export(
        "events",
        &tree.roots,
        render_event_binding,
    ))
}

pub(super) fn render_sdk_aggregate(
    functions: &ObjectTree<SdkFunction>,
    domain_names: &BTreeMap<String, String>,
    contexts: &ObjectTree<SdkContext>,
    context_names: &BTreeMap<String, String>,
    events: &ObjectTree<SdkEvent>,
    models: &BTreeMap<String, TsType>,
) -> String {
    let mut entries = Vec::new();

    if !models.is_empty() {
        entries.push(format!("{}models,", indent(1)));
    }
    if !functions.is_empty() {
        entries.push(format!(
            "{}functions: {},",
            indent(1),
            render_export_reference_object(&functions.roots, domain_names, 1)
        ));
        entries.push(format!("{}call,", indent(1)));
    }
    if !events.is_empty() {
        entries.push(format!("{}events,", indent(1)));
    }
    entries.push(format!("{}ctx,", indent(1)));
    if !contexts.is_empty() {
        entries.push(format!(
            "{}contexts: {},",
            indent(1),
            render_export_reference_object(&contexts.roots, context_names, 1)
        ));
    }

    format!(
        "export const manivelleSdk = {{\n{}\n}};",
        entries.join("\n")
    )
}

pub(super) fn render_host_function_api(functions: &BTreeMap<String, SdkFunctionType>) -> String {
    let host_functions = render_host_function_type_map(functions);
    format!(
        "{host_functions}\n\n\
type HostFunctionInput<K extends keyof HostFunctions> = HostFunctions[K][\"input\"];\n\
type HostFunctionOutput<K extends keyof HostFunctions> = HostFunctions[K][\"output\"];\n\
type HostFunctionArgs<K extends keyof HostFunctions> = HostFunctionInput<K> extends void ? [] : [input: HostFunctionInput<K>];\n\n\
export function call<K extends keyof HostFunctions>(name: K, ...args: HostFunctionArgs<K>): HostFunctionOutput<K> {{\n\
  return __hostCall<HostFunctionOutput<K>>(name, args[0]);\n\
}}"
    )
}

fn render_host_function_type_map(functions: &BTreeMap<String, SdkFunctionType>) -> String {
    let entries = functions
        .iter()
        .map(|(name, function)| {
            format!(
                "  {name:?}: {{ input: {}; output: {}; }};",
                function.input_type, function.output_type,
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    format!("type HostFunctions = {{\n{entries}\n}};")
}

fn render_export_reference_object<T>(
    roots: &BTreeMap<String, ObjectNode<T>>,
    names: &BTreeMap<String, String>,
    depth: usize,
) -> String {
    let entries = roots
        .keys()
        .map(|name| {
            format!(
                "{}{}: {},",
                indent(depth + 1),
                property_name(name),
                names[name]
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    if entries.is_empty() {
        String::from("{}")
    } else {
        format!("{{\n{entries}\n{}}}", indent(depth))
    }
}

fn render_exported_tree<T>(
    tree: &ObjectTree<T>,
    names: &BTreeMap<String, String>,
    render_binding: fn(&str, &T, usize) -> String,
) -> Vec<String> {
    tree.roots
        .iter()
        .map(|(name, node)| {
            let body = render_object_node(node, 1, render_binding);
            format!("export const {} = {body};", names[name])
        })
        .collect()
}

fn render_named_object_export<T>(
    export_name: &str,
    roots: &BTreeMap<String, ObjectNode<T>>,
    render_binding: fn(&str, &T, usize) -> String,
) -> String {
    let body = render_object_entries(roots, 1, render_binding);
    format!("export const {export_name} = {{\n{body}\n}};")
}

fn render_object_node<T>(
    node: &ObjectNode<T>,
    depth: usize,
    render_binding: fn(&str, &T, usize) -> String,
) -> String {
    let body = render_node_entries(node, depth, render_binding);
    if body.is_empty() {
        String::from("{}")
    } else {
        format!("{{\n{body}\n{}}}", indent(depth - 1))
    }
}

fn render_node_entries<T>(
    node: &ObjectNode<T>,
    depth: usize,
    render_binding: fn(&str, &T, usize) -> String,
) -> String {
    let mut entries = Vec::new();

    if let Some(binding) = &node.binding {
        entries.push(render_binding("default", binding, depth));
    }

    entries.extend(
        render_object_entries(&node.children, depth, render_binding)
            .lines()
            .map(str::to_owned),
    );
    entries.join("\n")
}

fn render_object_entries<T>(
    nodes: &BTreeMap<String, ObjectNode<T>>,
    depth: usize,
    render_binding: fn(&str, &T, usize) -> String,
) -> String {
    nodes
        .iter()
        .map(|(name, node)| {
            if let (Some(binding), true) = (&node.binding, node.children.is_empty()) {
                render_binding(name, binding, depth)
            } else {
                render_namespace_entry(name, node, depth, render_binding)
            }
        })
        .collect::<Vec<_>>()
        .join("\n")
}

fn render_namespace_entry<T>(
    name: &str,
    node: &ObjectNode<T>,
    depth: usize,
    render_binding: fn(&str, &T, usize) -> String,
) -> String {
    let child_body = render_node_entries(node, depth + 1, render_binding);
    format!(
        "{}{}: {{\n{}\n{}}},",
        indent(depth),
        property_name(name),
        child_body,
        indent(depth)
    )
}

fn render_function_binding(name: &str, function: &SdkFunction, depth: usize) -> String {
    let parameter = function_parameter(function);
    let call = format!(
        "return __hostCall<{}>({:?}{});",
        function.output_type,
        function.contract_name,
        function_input_argument(function)
    );

    let docs = function
        .docs
        .as_deref()
        .map(|docs| render_doc_comment(docs, &indent(depth)))
        .unwrap_or_default();
    format!(
        "{docs}{}{}({}): {} {{\n{}{}\n{}}},",
        indent(depth),
        property_name(name),
        parameter,
        function.output_type,
        indent(depth + 1),
        call,
        indent(depth)
    )
}

fn function_parameter(function: &SdkFunction) -> String {
    if function.takes_input {
        format!("input: {}", function.input_type)
    } else {
        String::new()
    }
}

fn function_input_argument(function: &SdkFunction) -> String {
    if function.takes_input {
        String::from(", input")
    } else {
        String::new()
    }
}

fn render_event_binding(name: &str, event: &SdkEvent, depth: usize) -> String {
    let docs = event
        .docs
        .as_deref()
        .map(|docs| render_doc_comment(docs, &indent(depth)))
        .unwrap_or_default();
    format!(
        "{docs}{}{}(handler: HostEventHandler<{:?}>): void {{\n{}return __ctx.on({:?}, handler);\n{}}},",
        indent(depth),
        property_name(name),
        event.event_name,
        indent(depth + 1),
        event.event_name,
        indent(depth)
    )
}

fn render_domain_binding(name: &str, binding: &SdkDomainBinding, depth: usize) -> String {
    match binding {
        SdkDomainBinding::Function(function) => render_function_binding(name, function, depth),
        SdkDomainBinding::EventAlias(event) => render_event_alias_binding(name, event, depth),
    }
}

fn render_event_alias_binding(name: &str, event: &SdkEventAlias, depth: usize) -> String {
    format!(
        "{}{}(handler: HostEventHandler<{:?}>): void {{\n{}return __ctx.on({:?}, handler);\n{}}},",
        indent(depth),
        property_name(name),
        event.event_name,
        indent(depth + 1),
        event.event_name,
        indent(depth)
    )
}

pub(super) fn push_if_some(sections: &mut Vec<String>, section: Option<String>) {
    if let Some(section) = section
        && !section.trim().is_empty()
    {
        sections.push(section);
    }
}
