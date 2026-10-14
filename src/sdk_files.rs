//! Helpers for writing generated TypeScript SDK artifacts.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use crate::contract::HostContractDescriptor;
use crate::error::EngineError;
use crate::registry::{
    InMemoryHostContractRegistry, render_typescript_declarations_for_descriptors,
    render_typescript_sdk_for_descriptors,
};

/// The environment variable through which the `manivelle` tool asks a program to export
/// its SDK: it holds the directory to write to. See [`export_sdk_if_requested`].
pub const EXPORT_SDK_ENV: &str = "MANIVELLE_EXPORT_SDK";

/// The file [`export_sdk_if_requested`] writes next to the generated TypeScript files: the
/// host contract descriptors, as JSON.
pub const DESCRIPTORS_FILE: &str = "descriptors.json";

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
) -> Result<GeneratedSdkFiles, EngineError> {
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
) -> Result<GeneratedSdkFiles, EngineError> {
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

/// Lets a program hand its scripting API to the `manivelle` tool, so that
/// `manivelle init --from your-program` can write the declarations a mod author needs.
///
/// Call it once, right after registering the host contracts and before doing anything
/// else the program does at startup (opening a window, loading assets):
///
/// ```no_run
/// # fn main() -> Result<(), manivelle::EngineError> {
/// let engine = manivelle::Engine::new(&manivelle::EngineOptions::default())?;
/// // engine.registry().function::<MyContract>()?; ...
/// if manivelle::export_sdk_if_requested(engine.registry())? {
///     return Ok(()); // the tool asked for the SDK: it is written, nothing else to do
/// }
/// # Ok(())
/// # }
/// ```
///
/// When the tool started the program, [`EXPORT_SDK_ENV`] names a directory: the function
/// writes the declarations (`manivelle.d.ts`), the SDK module (`manivelle.sdk.ts`) and
/// [`DESCRIPTORS_FILE`] there and returns `true`, and the program should exit. Otherwise
/// it does nothing and returns `false`. The program opts in by calling it; nothing
/// reads the executable from outside.
///
/// # Errors
///
/// Returns an error if the contracts cannot be listed or a file cannot be written.
pub fn export_sdk_if_requested(
    registry: &InMemoryHostContractRegistry,
) -> Result<bool, EngineError> {
    let Some(directory) = std::env::var_os(EXPORT_SDK_ENV).filter(|value| !value.is_empty()) else {
        return Ok(false);
    };
    let directory = PathBuf::from(directory);
    let descriptors = registry.descriptors()?;
    write_host_sdk_files(&directory, &descriptors)?;
    let json = serde_json::to_vec_pretty(&descriptors).map_err(io::Error::other)?;
    fs::write(directory.join(DESCRIPTORS_FILE), json)?;
    Ok(true)
}
