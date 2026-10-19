mod builder;
mod models;
mod predicates;
mod tree_render;

use crate::contract::HostContractDescriptor;

use super::declarations::push_descriptor;
use builder::SdkBuilder;

const HOST_HELPERS: &str = include_str!("../../../assets/sdk_host_helpers.ts");

/// The first line of every generated SDK.
pub(crate) const GENERATED_SDK_MARKER: &str = "// manivelle: generated";

pub(crate) fn render_typescript_sdk(descriptors: &[HostContractDescriptor]) -> String {
    let mut builder = SdkBuilder::default();

    for descriptor in descriptors {
        push_descriptor(&mut builder, descriptor);
    }

    builder.finish()
}
