//! Host contract traits and metadata.

mod arity;
mod base;
mod bridge;
mod callback;
mod caller;
mod codec;
mod context;
mod function;
mod host_resolver;
mod metadata;
mod native_bytes;
mod schema;
mod script_export;
pub(crate) mod validation;

pub use base::HostContract;
pub(crate) use bridge::{js_to_json, js_to_snapshot, json_to_js};
pub use callback::{HostCallback, HostRequest};
pub use caller::Caller;
pub use codec::{CodecError, JsArgs, JsDecode, JsEncode, MapKey, PathSegment};
pub use codec::{derive, expect_array_len, expect_object};
pub use context::HostContext;
pub use function::{HostFunction, HostFunctionSignature};
pub use host_resolver::HostResolver;
pub use metadata::{
    HostCallbackDescriptor, HostContractAbi, HostContractDescriptor, HostContractKind,
    HostFunctionDescriptor, HostImportBinding, HostMetadata, Schema, TsEnumVariant, TsField,
    TsLiteral, TsRecordKey, TsType,
};
pub use native_bytes::NativeBytes;
pub use schema::{ObjectSchema, TsSchema, push_schema_dependency, schema_type_ref};
pub use script_export::{ScriptExport, TsArgs};
