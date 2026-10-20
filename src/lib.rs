//! Embedded TypeScript for Rust hosts: `oxc` transpiles, QuickJS (`rquickjs`) runs the
//! scripts on the host's own thread, and typed host contracts generate the scripts'
//! TypeScript declarations.

#![deny(clippy::correctness)]
#![warn(clippy::suspicious)]
#![warn(clippy::style)]
#![warn(clippy::complexity)]
#![warn(clippy::perf)]
#![warn(clippy::undocumented_unsafe_blocks)]
// A clone of a reference-counted pointer says so (`Rc::clone(&x)`), so a reader sees at a
// glance that no data is copied.
#![warn(clippy::clone_on_ref_ptr)]
#![warn(missing_docs)]

mod analysis;
mod cache;
mod compiler;
mod config;
mod contract;
mod error;
pub mod js;
mod packages;
pub mod protocol;
mod registry;
mod runner;
mod sdk_files;
mod types;

pub use analysis::{
    ContractNames, Finding, PackageReport, ScriptUsage, Severity, Usage, analyze_project,
    analyze_source, check_package,
};
pub use config::{ContractValidation, EngineOptions, ScriptBuiltins, UnknownFieldValidation};
pub use contract::{
    Caller, CodecError, HostFunctionSignature, HostRequest, HostResolver, JsArgs, JsDecode,
    JsEncode, PathSegment, ScriptExport, TsArgs,
};
pub use contract::{
    HostCallback, HostCallbackDescriptor, HostContext, HostContract, HostContractAbi,
    HostContractDescriptor, HostContractKind, HostFunction, HostFunctionDescriptor, HostMetadata,
    MapKey, NativeBytes, Schema, TsEnumVariant, TsField, TsLiteral, TsRecordKey, TsSchema, TsType,
};
/// Items the procedural macros generate code against. Not part of the public API.
#[doc(hidden)]
pub mod __private {
    pub use crate::contract::{
        ObjectSchema, derive, expect_array_len, expect_object, push_schema_dependency,
        schema_type_ref,
    };
    pub use serde;
}
pub use error::{Direction, EngineError};
#[cfg(feature = "derive")]
pub use manivelle_macros::{
    TsSchema, host_event, host_fn, host_fn_async, host_fn_with_ctx, host_mod, script_export,
};
pub use packages::{
    MANIFEST_FILE, PackageDependency, PackageFailure, PackageInfo, PackageLoad, PackageManifest,
    add_permissions, resolve_packages,
};
pub use protocol::Setup;
pub use registry::{
    InMemoryHostContractRegistry, render_typescript_declarations_for_descriptors,
    render_typescript_sdk_for_descriptors,
};
pub use runner::{ConsoleLevel, Engine, InterruptHandle, PendingCall};
pub use sdk_files::{
    DESCRIPTORS_FILE, EXPORT_SDK_ENV, GeneratedSdkFiles, SdkFileNames, export_sdk_if_requested,
    write_host_sdk_files, write_host_sdk_files_with_names,
};
pub use types::{Emission, MemoryStats, ReloadReport, ScriptId, StateSnapshot};
