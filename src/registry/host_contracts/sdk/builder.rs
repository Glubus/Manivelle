use std::collections::BTreeMap;

use crate::contract::{Schema, TsType};

use super::super::declarations::{
    DescriptorSink, EVENT_CONTEXT_TYPE, HOT_CONTEXT_TYPE, SchemaEmitter, function_output_type,
    render_doc_comment, render_event_map, render_script_export, render_script_exports_type,
    schema_type_name,
};
use super::super::names::binding_names;
use super::super::tree::ObjectTree;
use super::models::{render_model_classes, render_model_helpers, render_model_runtime_helpers};
use super::tree_render::{
    domain_tree, push_if_some, render_context_exports, render_domain_exports, render_events_export,
    render_host_function_api, render_sdk_aggregate,
};
use super::{GENERATED_SDK_MARKER, HOST_HELPERS};

#[derive(Default)]
pub(super) struct SdkBuilder {
    schemas: SchemaEmitter,
    schema_sections: Vec<String>,
    functions: ObjectTree<SdkFunction>,
    events: ObjectTree<SdkEvent>,
    contexts: ObjectTree<SdkContext>,
    host_functions: BTreeMap<String, SdkFunctionType>,
    host_events: BTreeMap<String, String>,
    host_event_docs: BTreeMap<String, String>,
    host_replies: BTreeMap<String, String>,
    has_host_functions: bool,
    script_exports: Vec<String>,
}

impl DescriptorSink for SdkBuilder {
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
        self.has_host_functions = true;
        let output_type = function_output_type(output, returns_promise);
        self.host_functions.insert(
            name.to_owned(),
            SdkFunctionType {
                input_type: schema_type_name(input),
                output_type: output_type.clone(),
            },
        );
        self.functions.insert(
            name,
            SdkFunction {
                contract_name: name.to_owned(),
                input_type: schema_type_name(input),
                output_type,
                takes_input: !matches!(input.ts_type, TsType::Void),
                docs: docs.map(str::to_owned),
            },
        );
    }

    fn push_callback(
        &mut self,
        name: &str,
        payload: &Schema,
        reply: Option<&Schema>,
        docs: Option<&str>,
    ) {
        self.push_schema(payload);
        self.host_events
            .insert(name.to_owned(), schema_type_name(payload));
        if let Some(docs) = docs {
            self.host_event_docs
                .insert(name.to_owned(), docs.to_owned());
        }
        if let Some(reply) = reply {
            self.push_schema(reply);
            self.host_replies
                .insert(name.to_owned(), schema_type_name(reply));
        }
        self.events.insert(
            name,
            SdkEvent {
                event_name: name.to_owned(),
                docs: docs.map(str::to_owned),
            },
        );
    }

    fn push_context(&mut self, name: &str, schema: &Schema) {
        self.push_schema(schema);
        self.contexts.insert(
            name,
            SdkContext {
                contract_name: name.to_owned(),
                ty: schema_type_name(schema),
            },
        );
    }

    fn push_export(&mut self, name: &str, args: &[Schema], output: &Schema) {
        for schema in args.iter().chain(std::iter::once(output)) {
            self.push_schema(schema);
        }
        self.script_exports
            .push(render_script_export(name, args, output));
    }
}

impl SdkBuilder {
    fn push_schema(&mut self, schema: &Schema) {
        self.schemas.emit(schema, &mut self.schema_sections);
    }

    pub(super) fn finish(self) -> String {
        let host_helpers = self.host_helpers();
        let host_function_api = self.host_function_api();
        let host_event_types = self.host_event_types();
        let event_helpers = self.event_helpers();
        let ctx_export = self.ctx_export();
        let script_exports = self.script_exports_type();
        let domain = domain_tree(&self.functions, &self.events);
        let domain_names = binding_names(domain.roots.keys().map(String::as_str));
        let context_names = binding_names(self.contexts.roots.keys().map(String::as_str));
        let models = self.schemas.into_models();

        let mut sections = vec![GENERATED_SDK_MARKER.to_owned()];
        sections.extend(self.schema_sections);
        push_if_some(&mut sections, render_model_runtime_helpers(&models));
        sections.extend(render_model_classes(&models));
        push_if_some(&mut sections, render_model_helpers(&models));
        push_if_some(&mut sections, host_helpers);
        push_if_some(&mut sections, host_function_api);
        push_if_some(&mut sections, host_event_types);
        push_if_some(&mut sections, script_exports);
        push_if_some(&mut sections, event_helpers);
        sections.push(ctx_export);
        sections.extend(render_context_exports(&self.contexts, &context_names));
        sections.extend(render_domain_exports(&domain, &domain_names));
        push_if_some(&mut sections, render_events_export(&self.events));
        sections.push(render_sdk_aggregate(
            &self.functions,
            &domain_names,
            &self.contexts,
            &context_names,
            &self.events,
            &models,
        ));

        let mut output = sections.join("\n\n");
        output.push('\n');
        output
    }

    fn host_helpers(&self) -> Option<String> {
        self.has_host_functions
            .then(|| HOST_HELPERS.trim().to_owned())
    }

    fn host_event_types(&self) -> Option<String> {
        if self.host_events.is_empty() {
            return None;
        }

        let events: Vec<String> = self
            .host_events
            .iter()
            .map(|(event_name, ty)| {
                let docs = self
                    .host_event_docs
                    .get(event_name)
                    .map(|docs| render_doc_comment(docs, "  "))
                    .unwrap_or_default();
                format!("{docs}  {event_name:?}: {ty};")
            })
            .collect();
        let replies: Vec<String> = self
            .host_replies
            .iter()
            .map(|(event_name, ty)| format!("  {event_name:?}: {ty};"))
            .collect();
        Some(format!(
            "{}\n\n{}",
            render_event_map("HostEvents", &events),
            render_event_map("HostReplies", &replies),
        ))
    }

    fn script_exports_type(&self) -> Option<String> {
        if self.script_exports.is_empty() {
            return None;
        }
        let mut entries = self.script_exports.clone();
        entries.sort();
        Some(render_script_exports_type(&entries, true))
    }

    fn host_function_api(&self) -> Option<String> {
        if self.host_functions.is_empty() {
            return None;
        }

        Some(render_host_function_api(&self.host_functions))
    }

    fn event_helpers(&self) -> Option<String> {
        (!self.host_events.is_empty()).then(|| EVENT_CONTEXT_TYPE.trim().to_owned())
    }

    /// The `ctx` global: `ctx.hot` always, `ctx.on` and `ctx.off` once there are events
    /// to type them.
    fn ctx_export(&self) -> String {
        let events = if self.host_events.is_empty() {
            ""
        } else {
            "HostEventContext & "
        };
        format!(
            "{}\n\ntype HostContext = {events}{{ readonly hot: HostHotContext }};\n\nconst __ctx = (globalThis as unknown as {{ ctx: HostContext }}).ctx;\n\nexport const ctx = __ctx;",
            HOT_CONTEXT_TYPE.trim()
        )
    }
}

#[derive(Clone)]
pub(super) struct SdkFunction {
    pub(super) contract_name: String,
    pub(super) input_type: String,
    pub(super) output_type: String,
    pub(super) takes_input: bool,
    pub(super) docs: Option<String>,
}

pub(super) struct SdkFunctionType {
    pub(super) input_type: String,
    pub(super) output_type: String,
}

pub(super) struct SdkEvent {
    pub(super) event_name: String,
    pub(super) docs: Option<String>,
}

#[derive(Clone)]
pub(super) struct SdkEventAlias {
    pub(super) event_name: String,
}

pub(super) enum SdkDomainBinding {
    Function(SdkFunction),
    EventAlias(SdkEventAlias),
}

pub(super) struct SdkContext {
    pub(super) contract_name: String,
    pub(super) ty: String,
}
