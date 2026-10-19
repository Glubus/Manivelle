//! Host contract registry.

mod host_contracts;

pub(crate) use host_contracts::{GENERATED_SDK_MARKER, HostModuleStyle, NativeContext};

pub use host_contracts::{
    InMemoryHostContractRegistry, render_typescript_declarations_for_descriptors,
    render_typescript_sdk_for_descriptors,
};
