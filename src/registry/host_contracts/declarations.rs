use std::collections::{BTreeMap, BTreeSet};

use crate::contract::{
    HostContractAbi, HostContractDescriptor, Schema, TsEnumVariant, TsField, TsLiteral,
    TsRecordKey, TsType,
};

use super::names::{indent, is_identifier_name};

const DEFAULT_ENUM_TAG: &str = "type";

/// `HostHotContext`, the type of `ctx.hot`, shared by the declarations and the SDK.
pub(super) const HOT_CONTEXT_TYPE: &str = include_str!("../../../assets/hot_context.ts");

/// `ctx.on` / `ctx.off` and the handler types, shared by the declarations and the SDK.
pub(super) const EVENT_CONTEXT_TYPE: &str = include_str!("../../../assets/event_context.ts");

/// The `manivelle:env` module, whose `ctx` has the type `__CTX__`.
pub(super) const ENV_MODULE_TYPE: &str = include_str!("../../../assets/env_module.ts");

pub(crate) fn render_typescript_declarations(descriptors: &[HostContractDescriptor]) -> String {
    let mut declarations = DeclarationBuffer::default();

    for descriptor in descriptors {
        push_descriptor(&mut declarations, descriptor);
    }

    declarations.finish()
}

/// Receives the contracts of a registry by ABI kind. The declarations and the SDK render
/// each kind from these calls, so both see the same dispatch.
pub(super) trait DescriptorSink {
    fn push_function(
        &mut self,
        name: &str,
        input: &Schema,
        output: &Schema,
        returns_promise: bool,
        docs: Option<&str>,
    );
    fn push_callback(
        &mut self,
        name: &str,
        payload: &Schema,
        reply: Option<&Schema>,
        docs: Option<&str>,
    );
    fn push_context(&mut self, name: &str, schema: &Schema);
    fn push_export(&mut self, name: &str, args: &[Schema], output: &Schema);
}

/// Hands `descriptor` to the method of `sink` for its ABI; `Unknown` contracts have no
/// declaration.
pub(super) fn push_descriptor(sink: &mut impl DescriptorSink, descriptor: &HostContractDescriptor) {
    match &descriptor.abi {
        HostContractAbi::Function {
            input,
            output,
            returns_promise,
        } => {
            sink.push_function(
                &descriptor.name,
                input,
                output,
                *returns_promise,
                descriptor.docs.as_deref(),
            );
        }
        HostContractAbi::Callback { payload, reply } => {
            sink.push_callback(
                &descriptor.name,
                payload,
                reply.as_ref(),
                descriptor.docs.as_deref(),
            );
        }
        HostContractAbi::Context { schema } => sink.push_context(&descriptor.name, schema),
        HostContractAbi::Export { args, output } => {
            sink.push_export(&descriptor.name, args, output);
        }
        HostContractAbi::Unknown => {}
    }
}

/// The `type` aliases of the schemas a registry uses: each name once, dependencies first.
#[derive(Default)]
pub(super) struct SchemaEmitter {
    emitted: BTreeSet<String>,
    /// The object schemas, which the SDK renders as model classes.
    models: BTreeMap<String, TsType>,
}

impl SchemaEmitter {
    /// Appends the alias of `schema` to `sections`, after those of its dependencies. A
    /// schema that is unknown or already emitted adds nothing.
    pub(super) fn emit(&mut self, schema: &Schema, sections: &mut Vec<String>) {
        let Some(name) = schema.declared_name() else {
            return;
        };
        if !self.emitted.insert(name.to_owned()) {
            return;
        }

        for dependency in &schema.dependencies {
            self.emit(dependency, sections);
        }

        if matches!(
            schema.ts_type,
            TsType::Object(_) | TsType::OpenObject { .. }
        ) {
            self.models.insert(name.to_owned(), schema.ts_type.clone());
        }

        sections.push(format!(
            "type {name} = {};",
            render_ts_type(&schema.ts_type)
        ));
    }

    pub(super) fn into_models(self) -> BTreeMap<String, TsType> {
        self.models
    }
}

#[derive(Default)]
struct DeclarationBuffer {
    sections: Vec<String>,
    host_events: Vec<(String, String)>,
    host_replies: Vec<String>,
    script_exports: Vec<String>,
    schemas: SchemaEmitter,
}

impl DeclarationBuffer {
    fn push_schema(&mut self, schema: &Schema) {
        self.schemas.emit(schema, &mut self.sections);
    }
}

impl DescriptorSink for DeclarationBuffer {
    fn push_function(
        &mut self,
        name: &str,
        input: &Schema,
        output: &Schema,
        returns_promise: bool,
        docs: Option<&str>,
    ) {
        self.push_schema(input);
        self.push_schema(output);
        self.sections.push(render_function_declaration(
            name,
            input,
            &function_output_type(output, returns_promise),
            docs,
        ));
    }

    fn push_callback(
        &mut self,
        name: &str,
        payload: &Schema,
        reply: Option<&Schema>,
        docs: Option<&str>,
    ) {
        self.push_schema(payload);
        let docs = docs
            .map(|docs| render_doc_comment(docs, "  "))
            .unwrap_or_default();
        self.host_events.push((
            name.to_owned(),
            format!("{docs}  {name:?}: {};", schema_type_name(payload)),
        ));
        if let Some(reply) = reply {
            self.push_schema(reply);
            self.host_replies
                .push(format!("  {name:?}: {};", schema_type_name(reply)));
        }
    }

    fn push_context(&mut self, name: &str, schema: &Schema) {
        self.push_schema(schema);
        self.sections.push(format!(
            "declare const {name}: {};",
            schema_type_name(schema)
        ));
    }

    fn push_export(&mut self, name: &str, args: &[Schema], output: &Schema) {
        for schema in args.iter().chain(std::iter::once(output)) {
            self.push_schema(schema);
        }
        self.script_exports
            .push(render_script_export(name, args, output));
    }
}

impl DeclarationBuffer {
    fn finish(mut self) -> String {
        self.push_host_events();
        self.push_script_exports();
        self.push_ctx();
        let mut output = self.sections.join("\n\n");
        output.push('\n');
        output
    }

    fn push_host_events(&mut self) {
        if self.host_events.is_empty() {
            return;
        }

        self.host_events.sort_by(|a, b| a.0.cmp(&b.0));
        self.host_replies.sort();
        let events: Vec<String> = self
            .host_events
            .iter()
            .map(|(_, entry)| entry.clone())
            .collect();
        self.sections.push(render_event_map("HostEvents", &events));
        self.sections
            .push(render_event_map("HostReplies", &self.host_replies));
    }

    fn push_script_exports(&mut self) {
        if self.script_exports.is_empty() {
            return;
        }
        self.script_exports.sort();
        self.sections
            .push(render_script_exports_type(&self.script_exports, false));
    }

    /// The `ctx` global: `ctx.hot` always, `ctx.on` and `ctx.off` once there are events
    /// to type them.
    fn push_ctx(&mut self) {
        let events = if self.host_events.is_empty() {
            ""
        } else {
            self.sections.push(EVENT_CONTEXT_TYPE.trim().to_owned());
            "HostEventContext & "
        };
        self.sections.push(HOT_CONTEXT_TYPE.trim().to_owned());
        let ctx_type = format!("{events}{{ readonly hot: HostHotContext }}");
        self.sections
            .push(format!("declare const ctx: {ctx_type};"));
        self.sections
            .push(ENV_MODULE_TYPE.trim().replace("__CTX__", &ctx_type));
    }
}

/// `type {name} = { ... };` from `"event": Type;` entries, `{}` without any.
pub(super) fn render_event_map(name: &str, entries: &[String]) -> String {
    if entries.is_empty() {
        return format!("type {name} = {{}};");
    }
    format!("type {name} = {{\n{}\n}};", entries.join("\n"))
}

/// `  "name": (arg0: A, arg1: B) => Output | Promise<Output>;`, one entry of
/// `ScriptExports`: an export may be `async`.
pub(super) fn render_script_export(name: &str, args: &[Schema], output: &Schema) -> String {
    let parameters = args
        .iter()
        .enumerate()
        .map(|(index, arg)| format!("arg{index}: {}", schema_type_name(arg)))
        .collect::<Vec<_>>()
        .join(", ");
    let output = schema_type_name(output);
    format!("  {name:?}: ({parameters}) => {output} | Promise<{output}>;")
}

/// The `ScriptExports` type from the entries [`render_script_export`] made; `export` makes
/// it a module export, as in the SDK.
pub(super) fn render_script_exports_type(entries: &[String], export: bool) -> String {
    let keyword = if export { "export " } else { "" };
    format!(
        "/**\n * The functions the host calls on a script. Annotate an export with its entry for\n * TypeScript to check it: `export const run: ScriptExports[\"run\"] = (arg0) => ...`.\n */\n{keyword}type ScriptExports = {{\n{}\n}};",
        entries.join("\n")
    )
}

fn render_function_declaration(
    name: &str,
    input: &Schema,
    output_type: &str,
    docs: Option<&str>,
) -> String {
    let name_parts = split_contract_name(name);
    let signature = format!(
        "function {}(input: {}): {output_type};",
        name_parts.function_name,
        schema_type_name(input),
    );

    if name_parts.namespaces.is_empty() {
        let docs = docs
            .map(|docs| render_doc_comment(docs, ""))
            .unwrap_or_default();
        return format!("{docs}declare {signature}");
    }

    render_nested_namespace(&name_parts.namespaces, &signature, docs)
}

struct ContractNameParts<'a> {
    namespaces: Vec<&'a str>,
    function_name: &'a str,
}

fn split_contract_name(name: &str) -> ContractNameParts<'_> {
    let mut segments = name.split('.').collect::<Vec<_>>();
    let function_name = segments.pop().unwrap_or(name);
    ContractNameParts {
        namespaces: segments,
        function_name,
    }
}

fn render_nested_namespace(namespaces: &[&str], signature: &str, docs: Option<&str>) -> String {
    let mut output = String::new();

    for (depth, namespace) in namespaces.iter().enumerate() {
        output.push_str(&indent(depth));
        if depth == 0 {
            output.push_str("declare ");
        }
        output.push_str("namespace ");
        output.push_str(namespace);
        output.push_str(" {\n");
    }

    if let Some(docs) = docs {
        output.push_str(&render_doc_comment(docs, &indent(namespaces.len())));
    }
    output.push_str(&indent(namespaces.len()));
    output.push_str("export ");
    output.push_str(signature);
    output.push('\n');

    for depth in (0..namespaces.len()).rev() {
        output.push_str(&indent(depth));
        output.push('}');
        if depth > 0 {
            output.push('\n');
        }
    }

    output
}

/// `docs` as a JSDoc block at `indent`, ending with a newline; nothing for empty docs. A
/// `*/` inside the text cannot end the comment.
pub(super) fn render_doc_comment(docs: &str, indent: &str) -> String {
    let lines: Vec<String> = docs
        .trim()
        .lines()
        .map(|line| line.trim_end().replace("*/", "*\\/"))
        .collect();
    match lines.as_slice() {
        [] => String::new(),
        [line] => format!("{indent}/** {line} */\n"),
        lines => {
            let mut block = format!("{indent}/**\n");
            for line in lines {
                if line.is_empty() {
                    block.push_str(&format!("{indent} *\n"));
                } else {
                    block.push_str(&format!("{indent} * {line}\n"));
                }
            }
            block.push_str(&format!("{indent} */\n"));
            block
        }
    }
}

pub(super) fn schema_type_name(schema: &Schema) -> String {
    match schema.declared_name() {
        Some(name) => name.to_owned(),
        None => render_ts_type(&schema.ts_type),
    }
}

/// What a host function returns to scripts: its output, or a `Promise` of it.
pub(super) fn function_output_type(output: &Schema, returns_promise: bool) -> String {
    let output = schema_type_name(output);
    if returns_promise {
        format!("Promise<{output}>")
    } else {
        output
    }
}

pub(super) fn render_ts_type(ty: &TsType) -> String {
    match ty {
        TsType::Unknown => String::from("unknown"),
        TsType::Void => String::from("void"),
        TsType::Boolean => String::from("boolean"),
        TsType::Number => String::from("number"),
        TsType::String => String::from("string"),
        TsType::Json => String::from("unknown"),
        TsType::Uint8Array => String::from("Uint8Array"),
        TsType::Null => String::from("null"),
        TsType::TypeRef(name) => name.clone(),
        TsType::Literal(literal) => render_literal(literal),
        TsType::Object(fields) => render_object_type(fields.iter(), None),
        TsType::OpenObject { fields, rest } => {
            render_object_type(fields.iter(), Some(rest.as_ref()))
        }
        TsType::Array(item) => format!("{}[]", render_wrapped_array_type(item)),
        TsType::Tuple(items) => render_tuple_type(items),
        TsType::Enum { tag, variants } => render_enum_type(tag.as_deref(), variants),
        TsType::Record { key, value } => render_record_type(*key, value),
        TsType::Union(types) => render_union_type(types),
        TsType::Optional(inner) => format!("{} | undefined", render_ts_type(inner)),
        TsType::Nullable(inner) => format!("{} | null", render_ts_type(inner)),
    }
}

fn render_literal(literal: &TsLiteral) -> String {
    match literal {
        TsLiteral::String(value) => render_string_literal(value),
        TsLiteral::Number(value) => value.clone(),
        TsLiteral::Boolean(value) => value.to_string(),
    }
}

fn render_string_literal(value: &str) -> String {
    format!("{value:?}")
}

fn render_object_type<'a>(
    fields: impl Iterator<Item = &'a TsField> + Clone,
    rest: Option<&TsType>,
) -> String {
    let mut members = fields.clone().map(render_field).collect::<Vec<_>>();
    if let Some(rest) = rest {
        members.push(render_index_signature(fields, rest));
    }
    if members.is_empty() {
        return String::from("{}");
    }
    format!("{{ {} }}", members.join(" "))
}

fn render_field(field: &TsField) -> String {
    let optional = if field.optional { "?" } else { "" };
    format!(
        "{}{}: {};",
        render_property_key(&field.name),
        optional,
        render_ts_type(&field.ty)
    )
}

/// `name` as a property key in a type literal: bare when it is an identifier (reserved
/// words included, which are legal there), a string literal otherwise.
fn render_property_key(name: &str) -> String {
    if is_identifier_name(name) {
        name.to_owned()
    } else {
        render_string_literal(name)
    }
}

/// `[key: string]: ...` admitting `rest` plus every declared field's type, because
/// TypeScript requires each property of the object to match its index signature.
fn render_index_signature<'a>(fields: impl Iterator<Item = &'a TsField>, rest: &TsType) -> String {
    let mut types = vec![render_ts_type(rest)];
    let mut any_optional = false;
    for field in fields {
        push_unique(&mut types, render_ts_type(&field.ty));
        any_optional |= field.optional;
    }
    if any_optional {
        push_unique(&mut types, String::from("undefined"));
    }
    format!("[key: string]: {};", types.join(" | "))
}

fn push_unique(types: &mut Vec<String>, ty: String) {
    if !types.contains(&ty) {
        types.push(ty);
    }
}

fn render_tuple_type(items: &[TsType]) -> String {
    let rendered_items = items
        .iter()
        .map(render_ts_type)
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{rendered_items}]")
}

fn render_enum_type(tag: Option<&str>, variants: &[TsEnumVariant]) -> String {
    if variants.is_empty() {
        return String::from("never");
    }

    if variants.iter().all(TsEnumVariant::is_unit) {
        return variants
            .iter()
            .map(|variant| render_string_literal(&variant.name))
            .collect::<Vec<_>>()
            .join(" | ");
    }

    let tag = tag.unwrap_or(DEFAULT_ENUM_TAG);
    variants
        .iter()
        .map(|variant| render_enum_variant_type(tag, variant))
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_enum_variant_type(tag: &str, variant: &TsEnumVariant) -> String {
    let discriminant = TsField::required(
        tag,
        TsType::Literal(TsLiteral::String(variant.name.clone())),
    );
    render_object_type(
        std::iter::once(&discriminant).chain(variant.fields.iter()),
        variant.rest.as_deref(),
    )
}

fn render_record_type(key: TsRecordKey, value: &TsType) -> String {
    let key_type = match key {
        TsRecordKey::String => "string",
        TsRecordKey::Number => "number",
    };
    format!("Record<{key_type}, {}>", render_ts_type(value))
}

fn render_union_type(types: &[TsType]) -> String {
    if types.is_empty() {
        return String::from("never");
    }

    types
        .iter()
        .map(render_ts_type)
        .collect::<Vec<_>>()
        .join(" | ")
}

fn render_wrapped_array_type(item: &TsType) -> String {
    match item {
        TsType::Object(_)
        | TsType::OpenObject { .. }
        | TsType::Enum { .. }
        | TsType::Optional(_)
        | TsType::Nullable(_)
        | TsType::Union(_)
        | TsType::Record { .. } => format!("({})", render_ts_type(item)),
        _ => render_ts_type(item),
    }
}
