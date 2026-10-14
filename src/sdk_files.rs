//! Helpers for writing generated TypeScript SDK artifacts.

use std::fs;
use std::path::{Path, PathBuf};

use crate::contract::HostContractDescriptor;
use crate::error::VmError;
use crate::registry::{
    render_typescript_declarations_for_descriptors, render_typescript_sdk_for_descriptors,
};

/// File names used when writing generated TypeScript artifacts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SdkFileNames {
    /// Declaration file name.
    pub types: String,
    /// SDK source file name.
    pub sdk: String,
}

impl Default for SdkFileNames {
    fn default() -> Self {
        Self {
            types: String::from("manivelle.d.ts"),
            sdk: String::from("manivelle.sdk.ts"),
        }
    }
}

/// Paths written by SDK artifact export helpers.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GeneratedSdkFiles {
    /// Path to the generated declaration file.
    pub types_path: PathBuf,
    /// Path to the generated SDK source file.
    pub sdk_path: PathBuf,
}

/// Writes TypeScript declarations and SDK source from host contract descriptors.
///
/// # Errors
///
/// Returns an error if the output directory cannot be created or either file cannot be written.
pub fn write_host_sdk_files(
    directory: impl AsRef<Path>,
    descriptors: &[HostContractDescriptor],
) -> Result<GeneratedSdkFiles, VmError> {
    write_host_sdk_files_with_names(directory, descriptors, &SdkFileNames::default())
}

/// Writes TypeScript declarations and SDK source with explicit output file names.
///
/// # Errors
///
/// Returns an error if the output directory cannot be created or either file cannot be written.
pub fn write_host_sdk_files_with_names(
    directory: impl AsRef<Path>,
    descriptors: &[HostContractDescriptor],
    names: &SdkFileNames,
) -> Result<GeneratedSdkFiles, VmError> {
    let directory = directory.as_ref();
    fs::create_dir_all(directory)?;

    let types_path = directory.join(&names.types);
    let sdk_path = directory.join(&names.sdk);

    fs::write(
        &types_path,
        render_typescript_declarations_for_descriptors(descriptors),
    )?;
    fs::write(
        &sdk_path,
        render_typescript_sdk_for_descriptors(descriptors),
    )?;

    Ok(GeneratedSdkFiles {
        types_path,
        sdk_path,
    })
}
