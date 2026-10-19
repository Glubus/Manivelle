//! Host contract metadata types.

use serde::{Deserialize, Serialize};

/// Minimal schema carrier for contract metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
pub struct Schema {
    /// Human-readable schema identity.
    pub name: String,
    /// TypeScript type shape represented by this schema.
    pub ts_type: TsType,
    /// Named schema dependencies referenced by this schema.
    pub dependencies: Vec<Schema>,
}

impl Schema {
    /// Creates a schema with an unknown TypeScript type.
    #[must_use]
    pub fn named(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            ts_type: TsType::Unknown,
            dependencies: Vec::new(),
        }
    }

    /// Creates a schema backed by a TypeScript type shape.
    #[must_use]
    pub fn typed(name: impl Into<String>, ts_type: TsType) -> Self {
        Self {
            name: name.into(),
            ts_type,
            dependencies: Vec::new(),
        }
    }

    /// Attaches named schema dependencies referenced by this schema.
    #[must_use]
    pub fn with_dependencies(mut self, dependencies: Vec<Schema>) -> Self {
        self.dependencies = dependencies;
        self
    }

    /// Validates one JSON value against this schema.
    ///
    /// Unknown object fields are allowed, matching the default host bridge
    /// validation policy.
    ///
    /// # Errors
    ///
    /// Returns a human-readable validation path and reason when the value does
    /// not match this schema.
    pub fn validate_json(&self, value: &serde_json::Value) -> Result<(), String> {
        super::validation::validate_schema_with_options(
            self,
            &super::validation::Snapshot::from(value),
            super::validation::SchemaValidationOptions {
                reject_unknown_fields: false,
            },
        )
    }

    /// Validates one JSON value against this schema and rejects unknown object fields.
    ///
    /// # Errors
    ///
    /// Returns a human-readable validation path and reason when the value does
    /// not match this schema or contains undeclared object fields.
    pub fn validate_json_strict(&self, value: &serde_json::Value) -> Result<(), String> {
        super::validation::validate_schema_with_options(
            self,
            &super::validation::Snapshot::from(value),
            super::validation::SchemaValidationOptions {
                reject_unknown_fields: true,
            },
        )
    }
}

/// TypeScript type shape emitted from host schemas.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq, Default)]
#[non_exhaustive]
pub enum TsType {
    /// Unknown type.
    #[default]
    Unknown,
    /// No value.
    Void,
    /// Boolean value.
    Boolean,
    /// Number value.
    Number,
    /// String value.
    String,
    /// Arbitrary JSON-compatible value.
    Json,
    /// Native byte buffer exposed to TypeScript as `Uint8Array`.
    Uint8Array,
    /// Null value.
    Null,
    /// Named TypeScript type reference.
    TypeRef(String),
    /// Literal TypeScript value.
    Literal(TsLiteral),
    /// Object type with named fields.
    Object(Vec<TsField>),
    /// Object type with named fields plus any other string key holding a `rest` value:
    /// the wire shape of a struct with a `#[serde(flatten)]` map.
    OpenObject {
        /// Declared fields.
        fields: Vec<TsField>,
        /// Type of the value under every undeclared key.
        rest: Box<TsType>,
    },
    /// Array type.
    Array(Box<TsType>),
    /// Tuple type.
    Tuple(Vec<TsType>),
    /// Rust-like enum type.
    Enum {
        /// Discriminant field used when variants carry payload fields.
        tag: Option<String>,
        /// Enum variants.
        variants: Vec<TsEnumVariant>,
    },
    /// Record type.
    Record {
        /// Record key type.
        key: TsRecordKey,
        /// Record value type.
        value: Box<TsType>,
    },
    /// Union type.
    Union(Vec<TsType>),
    /// Optional type.
    Optional(Box<TsType>),
    /// Nullable type.
    Nullable(Box<TsType>),
}

impl TsType {
    /// Object type with `fields`, open to undeclared keys of type `rest` when given.
    #[must_use]
    pub fn object(fields: Vec<TsField>, rest: Option<TsType>) -> Self {
        match rest {
            Some(rest) => Self::OpenObject {
                fields,
                rest: Box::new(rest),
            },
            None => Self::Object(fields),
        }
    }
}

/// Literal TypeScript value emitted from a schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub enum TsLiteral {
    /// String literal.
    String(String),
    /// Number literal stored as source text.
    Number(String),
    /// Boolean literal.
    Boolean(bool),
}

/// Variant of a TypeScript enum model.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TsEnumVariant {
    /// Variant name.
    pub name: String,
    /// Payload fields carried by this variant.
    pub fields: Vec<TsField>,
    /// Type of the value under every undeclared payload key, when the payload
    /// flattens a map.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub rest: Option<Box<TsType>>,
}

impl TsEnumVariant {
    /// Creates a unit enum variant.
    #[must_use]
    pub fn unit(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            fields: Vec::new(),
            rest: None,
        }
    }

    /// Creates an enum variant with object payload fields.
    #[must_use]
    pub fn payload(name: impl Into<String>, fields: Vec<TsField>) -> Self {
        Self {
            name: name.into(),
            fields,
            rest: None,
        }
    }

    /// Opens the payload to undeclared keys of type `rest` when given.
    #[must_use]
    pub fn with_rest(mut self, rest: Option<TsType>) -> Self {
        self.rest = rest.map(Box::new);
        self
    }

    /// Whether this variant carries neither payload fields nor extra keys.
    #[must_use]
    pub fn is_unit(&self) -> bool {
        self.fields.is_empty() && self.rest.is_none()
    }
}

/// TypeScript record key category. The Rust side is [`MapKey`](crate::MapKey): every map
/// key type maps to one of these categories when the schema is emitted.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum TsRecordKey {
    /// String keys.
    String,
    /// Number keys.
    Number,
}

/// Object field in a TypeScript schema.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct TsField {
    /// Field name.
    pub name: String,
    /// Field type.
    pub ty: TsType,
    /// Whether this field is optional.
    pub optional: bool,
}

impl TsField {
    /// Creates a required field.
    #[must_use]
    pub fn required(name: impl Into<String>, ty: TsType) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: false,
        }
    }

    /// Creates an optional field.
    #[must_use]
    pub fn optional(name: impl Into<String>, ty: TsType) -> Self {
        Self {
            name: name.into(),
            ty,
            optional: true,
        }
    }
}

/// Shared host contract metadata.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostMetadata {
    /// Stable contract name.
    pub name: String,
    /// Free-form tags.
    pub tags: Vec<String>,
}

/// ESM import binding exposed by the host module loader.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostImportBinding {
    /// Virtual ESM module name, such as `"oppw4"`.
    pub module: String,
    /// Export path inside the module, such as `["character", "find"]`.
    pub export_path: Vec<String>,
}

/// Top-level contract category.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostContractKind {
    /// Function contract.
    Function,
    /// Callback contract.
    Callback,
    /// Context contract.
    Context,
    /// A function the script exports and the host calls; see
    /// [`ScriptExport`](crate::ScriptExport).
    Export,
}

/// Runtime and typing descriptor stored by the host contract registry.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostContractDescriptor {
    /// Stable contract name.
    pub name: String,
    /// Contract category.
    pub kind: HostContractKind,
    /// Schema metadata.
    pub schema: Schema,
    /// Extra metadata.
    pub metadata: HostMetadata,
    /// Required ESM import binding for scripts.
    pub import: HostImportBinding,
    /// Callback-specific routing metadata.
    pub callback: Option<HostCallbackDescriptor>,
    /// Function-specific call metadata.
    pub function: Option<HostFunctionDescriptor>,
    /// Normalized ABI contract model derived from Rust traits.
    pub abi: HostContractAbi,
    /// Documentation of the contract, rendered as JSDoc in the generated TypeScript.
    /// Serialized descriptors omit it when there is none.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub docs: Option<String>,
}

/// Metadata specific to callback contracts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostCallbackDescriptor {
    /// Payload schema metadata.
    pub payload_schema: Schema,
    /// What each handler returns, for a [`HostRequest`](crate::HostRequest); `None` for a
    /// plain callback, whose handlers return nothing.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reply_schema: Option<Schema>,
}

/// Typing metadata specific to function contracts.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
pub struct HostFunctionDescriptor {
    /// Input schema metadata.
    pub input_schema: Schema,
    /// Output schema metadata: what the function returns, or what its Promise resolves
    /// to when [`Self::returns_promise`] is set.
    pub output_schema: Schema,
    /// Whether scripts receive a `Promise` of the output, settled by a
    /// [`HostResolver`](crate::HostResolver). Serialized descriptors omit it when `false`.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pub returns_promise: bool,
}

/// Normalized ABI data for one host contract.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub enum HostContractAbi {
    /// Function ABI.
    Function {
        /// Input schema.
        input: Schema,
        /// Output schema, the value a Promise resolves to when `returns_promise` is set.
        output: Schema,
        /// Whether the function returns a `Promise` of the output.
        #[serde(default, skip_serializing_if = "std::ops::Not::not")]
        returns_promise: bool,
    },
    /// Callback ABI.
    Callback {
        /// Payload schema.
        payload: Schema,
        /// Handler reply schema of a request; `None` when handlers return nothing.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        reply: Option<Schema>,
    },
    /// Context ABI.
    Context {
        /// Context schema.
        schema: Schema,
    },
    /// A function a script exports.
    Export {
        /// The schema of each argument, in order.
        args: Vec<Schema>,
        /// What the export returns.
        output: Schema,
    },
    /// ABI has not been specialized yet.
    Unknown,
}
