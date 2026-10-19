//! Base host contract trait.

use super::{
    HostContractAbi, HostContractDescriptor, HostContractKind, HostImportBinding, HostMetadata,
    Schema,
};

/// Base contract trait. Traits produce the contract model, not just runtime hooks.
pub trait HostContract {
    /// Stable contract identity.
    const NAME: &'static str;

    /// Virtual ESM module name used by scripts.
    const IMPORT_MODULE: &'static str = "";

    /// Export path inside [`Self::IMPORT_MODULE`].
    const EXPORT_PATH: &'static [&'static str] = &[];

    /// Documentation shown on the generated TypeScript declarations and SDK, as
    /// Markdown. `#[host_fn]` fills it with the `///` comments of the function.
    const DOCS: &'static str = "";

    /// Returns additional host metadata.
    fn metadata() -> HostMetadata {
        HostMetadata {
            name: Self::NAME.to_owned(),
            tags: Vec::new(),
        }
    }

    /// Returns the category of this contract.
    fn kind() -> HostContractKind;

    /// Builds the base descriptor stored in registries. Its schema is a placeholder:
    /// registration fills it from the contract's types (`TsSchema`), or from
    /// [`HostContext::schema`](crate::HostContext::schema) for a context.
    fn descriptor() -> HostContractDescriptor {
        HostContractDescriptor {
            name: Self::NAME.to_owned(),
            kind: Self::kind(),
            schema: Schema::named("unknown"),
            metadata: Self::metadata(),
            import: HostImportBinding {
                module: Self::IMPORT_MODULE.to_owned(),
                export_path: Self::EXPORT_PATH
                    .iter()
                    .map(|segment| (*segment).to_owned())
                    .collect(),
            },
            callback: None,
            function: None,
            abi: HostContractAbi::Unknown,
            docs: (!Self::DOCS.is_empty()).then(|| Self::DOCS.to_owned()),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::HostContractKind;

    struct DemoContract;

    impl HostContract for DemoContract {
        const NAME: &'static str = "demo.contract";
        const IMPORT_MODULE: &'static str = "demo";
        const EXPORT_PATH: &'static [&'static str] = &["contract"];

        fn kind() -> HostContractKind {
            HostContractKind::Function
        }
    }

    #[test]
    fn descriptor_uses_contract_metadata() {
        let descriptor = DemoContract::descriptor();

        assert_eq!(descriptor.name, "demo.contract");
        assert_eq!(descriptor.kind, HostContractKind::Function);
    }
}
