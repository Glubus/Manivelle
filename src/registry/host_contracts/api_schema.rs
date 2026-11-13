//! The contracts of a registry as one JSON document, in the manner of OpenAPI: every
//! function, event and script export by name, with its documentation and the module
//! scripts import it from, and its types as JSON Schema (2020-12) under `schemas`,
//! referenced with `$ref`.
//!
//! It describes the same contracts as the TypeScript declarations, for readers that are
//! not a TypeScript compiler: documentation, tools, API discovery.

use serde_json::{Map, Value, json};

use super::declarations::DEFAULT_ENUM_TAG;
use crate::contract::{
    HostContractAbi, HostContractDescriptor, Schema, TsEnumVariant, TsField, TsLiteral,
    TsRecordKey, TsType,
};

/// The version of the document's layout, raised when a reader of the previous one would
/// misread the new one.
const VERSION: u32 = 1;

const DIALECT: &str = "https://json-schema.org/draft/2020-12/schema";

pub(crate) fn render_api_schema(descriptors: &[HostContractDescriptor]) -> Value {
    let mut document = Document::default();
    for descriptor in descriptors {
        document.push(descriptor);
    }
    json!({
        "manivelle": VERSION,
        "jsonSchemaDialect": DIALECT,
        "functions": document.functions,
        "events": document.events,
        "exports": document.exports,
        "contexts": document.contexts,
        "schemas": document.schemas,
    })
}

#[derive(Default)]
struct Document {
    functions: Map<String, Value>,
    events: Map<String, Value>,
    exports: Map<String, Value>,
    contexts: Map<String, Value>,
    /// The named types, each once.
    schemas: Map<String, Value>,
}

impl Document {
    fn push(&mut self, descriptor: &HostContractDescriptor) {
        let mut entry = Map::new();
        if let Some(docs) = &descriptor.docs {
            entry.insert("description".to_owned(), json!(docs));
        }
        if !descriptor.import.module.is_empty() {
            entry.insert("module".to_owned(), json!(descriptor.import.module));
            entry.insert("path".to_owned(), json!(descriptor.import.export_path));
        }
        let section = match &descriptor.abi {
            HostContractAbi::Function {
                input,
                output,
                returns_promise,
            } => {
                self.insert_value(&mut entry, "input", input);
                self.insert_value(&mut entry, "output", output);
                if *returns_promise {
                    entry.insert("async".to_owned(), json!(true));
                }
                &mut self.functions
            }
            HostContractAbi::Callback { payload, reply } => {
                self.insert_value(&mut entry, "payload", payload);
                if let Some(reply) = reply {
                    self.insert_value(&mut entry, "reply", reply);
                }
                &mut self.events
            }
            HostContractAbi::Context { schema } => {
                self.insert_value(&mut entry, "schema", schema);
                &mut self.contexts
            }
            HostContractAbi::Export { args, output } => {
                let args = args.iter().map(|arg| self.reference(arg)).collect();
                entry.insert("args".to_owned(), Value::Array(args));
                self.insert_value(&mut entry, "output", output);
                &mut self.exports
            }
            HostContractAbi::Unknown => return,
        };
        section.insert(descriptor.name.clone(), Value::Object(entry));
    }

    /// Sets `key` to the schema of a value a contract carries; a contract that carries
    /// nothing there (`void`) has no such key.
    fn insert_value(&mut self, entry: &mut Map<String, Value>, key: &str, schema: &Schema) {
        if schema.ts_type != TsType::Void {
            entry.insert(key.to_owned(), self.reference(schema));
        }
    }

    /// The schema of `schema` where it is used: a `$ref` to its entry in `schemas` when it
    /// has a name, recorded with its dependencies on first sight, its shape otherwise.
    fn reference(&mut self, schema: &Schema) -> Value {
        let name = schema.declared_name();
        if name.is_none_or(|name| !self.schemas.contains_key(name)) {
            if let Some(name) = name {
                self.schemas
                    .insert(name.to_owned(), json_schema(&schema.ts_type));
            }
            for dependency in &schema.dependencies {
                self.reference(dependency);
            }
        }
        name.map_or_else(|| json_schema(&schema.ts_type), type_ref)
    }
}

/// `$ref` to the named type `name`, as a JSON pointer into the document.
fn type_ref(name: &str) -> Value {
    let escaped = name.replace('~', "~0").replace('/', "~1");
    json!({ "$ref": format!("#/schemas/{escaped}") })
}

/// `ty` as a JSON Schema: what the value looks like once written as JSON.
fn json_schema(ty: &TsType) -> Value {
    match ty {
        TsType::Unknown | TsType::Json => json!({}),
        TsType::Void | TsType::Null => json!({ "type": "null" }),
        TsType::Boolean => json!({ "type": "boolean" }),
        TsType::Number => json!({ "type": "number" }),
        TsType::String => json!({ "type": "string" }),
        // A `Uint8Array` in the script; as JSON, the array of its bytes.
        TsType::Uint8Array => json!({
            "type": "array",
            "items": { "type": "integer", "minimum": 0, "maximum": 255 },
            "format": "uint8array",
        }),
        TsType::TypeRef(name) => type_ref(name),
        TsType::Literal(literal) => json!({ "const": literal_value(literal) }),
        TsType::Object(fields) => object_schema(None, fields, None),
        TsType::OpenObject { fields, rest } => object_schema(None, fields, Some(rest)),
        TsType::Array(item) => json!({ "type": "array", "items": json_schema(item) }),
        TsType::Tuple(items) => json!({
            "type": "array",
            "prefixItems": items.iter().map(json_schema).collect::<Vec<_>>(),
            "items": false,
            "minItems": items.len(),
        }),
        TsType::Enum { tag, variants } => enum_schema(tag.as_deref(), variants),
        TsType::Record { key, value } => {
            let mut schema = json!({
                "type": "object",
                "additionalProperties": json_schema(value),
            });
            if *key == TsRecordKey::Number {
                schema["propertyNames"] = json!({ "pattern": "^-?[0-9]+$" });
            }
            schema
        }
        TsType::Union(types) => json!({
            "anyOf": types.iter().map(json_schema).collect::<Vec<_>>(),
        }),
        // JSON has no `undefined`: outside an object, where the field is left out, it
        // is written `null`.
        TsType::Optional(inner) | TsType::Nullable(inner) => json!({
            "anyOf": [json_schema(inner), { "type": "null" }],
        }),
    }
}

fn literal_value(literal: &TsLiteral) -> Value {
    match literal {
        TsLiteral::String(text) => json!(text),
        // Stored as source text; one that is not a JSON number stays text.
        TsLiteral::Number(text) => serde_json::from_str(text).unwrap_or_else(|_| json!(text)),
        TsLiteral::Boolean(value) => json!(value),
    }
}

/// An object with `fields`, any other key holding a `rest` value when there is one, and
/// for an enum variant its `tag` set to the variant's name.
fn object_schema(tag: Option<(&str, &str)>, fields: &[TsField], rest: Option<&TsType>) -> Value {
    let mut properties = Map::new();
    let mut required = Vec::new();
    if let Some((tag, variant)) = tag {
        properties.insert(tag.to_owned(), json!({ "const": variant }));
        required.push(json!(tag));
    }
    for field in fields {
        // A field that may be `undefined` is one that may be absent.
        let (ty, optional) = match &field.ty {
            TsType::Optional(inner) => (inner.as_ref(), true),
            ty => (ty, field.optional),
        };
        properties.insert(field.name.clone(), json_schema(ty));
        if !optional {
            required.push(json!(field.name));
        }
    }
    let mut schema = json!({ "type": "object", "properties": properties });
    if !required.is_empty() {
        schema["required"] = Value::Array(required);
    }
    if let Some(rest) = rest {
        schema["additionalProperties"] = json_schema(rest);
    }
    schema
}

/// An enum as serde writes it: the variant names when none carries fields, otherwise one
/// object per variant, told apart by the `tag` field.
fn enum_schema(tag: Option<&str>, variants: &[TsEnumVariant]) -> Value {
    if variants.iter().all(TsEnumVariant::is_unit) {
        let names = variants.iter().map(|variant| &variant.name);
        return json!({ "enum": names.collect::<Vec<_>>() });
    }
    let tag = tag.unwrap_or(DEFAULT_ENUM_TAG);
    let variants = variants.iter().map(|variant| {
        object_schema(
            Some((tag, &variant.name)),
            &variant.fields,
            variant.rest.as_deref(),
        )
    });
    json!({ "oneOf": variants.collect::<Vec<_>>() })
}
