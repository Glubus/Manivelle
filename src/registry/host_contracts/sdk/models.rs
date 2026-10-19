use std::collections::BTreeMap;

use crate::contract::TsType;

use super::super::names::{indent, property_name};
use super::predicates::render_value_predicate;

pub(super) fn render_model_helpers(models: &BTreeMap<String, TsType>) -> Option<String> {
    if models.is_empty() {
        return None;
    }

    let entries = models
        .keys()
        .map(|name| render_model_helper_entry(name, 1))
        .collect::<Vec<_>>()
        .join("\n");

    Some(format!("export const models = {{\n{entries}\n}};"))
}

pub(super) fn render_model_classes(models: &BTreeMap<String, TsType>) -> Vec<String> {
    models.keys().map(|name| render_model_class(name)).collect()
}

pub(super) fn render_model_runtime_helpers(models: &BTreeMap<String, TsType>) -> Option<String> {
    if models.is_empty() {
        return None;
    }

    let predicates = models
        .iter()
        .map(|(name, ty)| render_model_predicate_function(name, ty))
        .collect::<Vec<_>>()
        .join("\n\n");

    Some(format!(
        "function __isRecord(value: unknown): value is Record<string, unknown> {{\n\
  return typeof value === \"object\" && value !== null && !Array.isArray(value);\n\
}}\n\n\
function __field(value: unknown, key: string): unknown {{\n\
  return __isRecord(value) ? value[key] : undefined;\n\
}}\n\n\
function __isArrayOf(value: unknown, predicate: (item: unknown) => boolean): boolean {{\n\
  return Array.isArray(value) && value.every(predicate);\n\
}}\n\n\
function __isRecordOf(value: unknown, predicate: (item: unknown, key: string) => boolean): boolean {{\n\
  return __isRecord(value) && Object.entries(value).every(([key, item]) => predicate(item, key));\n\
}}\n\n\
{predicates}"
    ))
}

fn render_model_predicate_function(name: &str, ty: &TsType) -> String {
    let predicate_name = model_predicate_name(name);
    let predicate = render_value_predicate(ty, "value");
    format!(
        "function {predicate_name}(value: unknown): value is {name} {{\n\
  return {predicate};\n\
}}"
    )
}

fn render_model_class(name: &str) -> String {
    let class_name = model_class_name(name);
    let predicate_name = model_predicate_name(name);
    format!(
        "export class {class_name} {{\n\
  constructor(public readonly value: {name}) {{}}\n\n\
  static create(value: {name}): {name} {{\n\
    return value;\n\
  }}\n\n\
  static is(value: unknown): value is {name} {{\n\
    return {predicate_name}(value);\n\
  }}\n\n\
  static wrap(value: {name}): {class_name} {{\n\
    return new {class_name}(value);\n\
  }}\n\n\
  toJSON(): {name} {{\n\
    return this.value;\n\
  }}\n\n\
  valueOf(): {name} {{\n\
    return this.value;\n\
  }}\n\
}}"
    )
}

fn render_model_helper_entry(name: &str, depth: usize) -> String {
    let class_name = model_class_name(name);
    let predicate_name = model_predicate_name(name);
    format!(
        "{}{}: {{\n{}create(value: {name}): {name} {{\n{}return {class_name}.create(value);\n{}}},\n{}is(value: unknown): value is {name} {{\n{}return {predicate_name}(value);\n{}}},\n{}wrap(value: {name}): {class_name} {{\n{}return {class_name}.wrap(value);\n{}}},\n{}}},",
        indent(depth),
        property_name(name),
        indent(depth + 1),
        indent(depth + 2),
        indent(depth + 1),
        indent(depth + 1),
        indent(depth + 2),
        indent(depth + 1),
        indent(depth + 1),
        indent(depth + 2),
        indent(depth + 1),
        indent(depth)
    )
}

fn model_predicate_name(name: &str) -> String {
    format!("__is{name}")
}

fn model_class_name(name: &str) -> String {
    format!("{name}Model")
}
