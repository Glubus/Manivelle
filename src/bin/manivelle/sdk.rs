//! `manivelle sdk`: writes the declarations and SDK module for a descriptors file.

use std::path::{Component, Path, PathBuf};

use clap::Args;
use manivelle::{SdkFileNames, write_host_sdk_files_with_names};

use super::Failure;
use super::export::read_descriptors;

#[derive(Args)]
pub struct SdkArgs {
    /// The descriptors file
    descriptors: PathBuf,
    /// Where to write the files
    output: PathBuf,
    #[command(flatten)]
    names: FileNames,
}

/// The names of the generated files, plain file names that cannot leave their directory.
#[derive(Args)]
pub struct FileNames {
    /// Declaration file name
    #[arg(long, default_value = "manivelle.d.ts", value_parser = declaration_name)]
    types: String,
    /// SDK module file name
    #[arg(long, default_value = "manivelle.sdk.ts", value_parser = module_name)]
    sdk: String,
}

impl FileNames {
    pub fn into_sdk_names(self) -> Result<SdkFileNames, Failure> {
        if self.types == self.sdk {
            return Err("--types and --sdk must name different files".into());
        }
        Ok(SdkFileNames {
            types: self.types,
            sdk: self.sdk,
        })
    }
}

fn declaration_name(value: &str) -> Result<String, String> {
    plain_file_name(value, ".d.ts")
}

fn module_name(value: &str) -> Result<String, String> {
    plain_file_name(value, ".ts")
}

/// `value` when it is one file name ending in `suffix`, with no directory in it: a name
/// like `../x.d.ts` or `/etc/x.d.ts` would write outside the project.
fn plain_file_name(value: &str, suffix: &str) -> Result<String, String> {
    let mut components = Path::new(value).components();
    let one_name = matches!(
        (components.next(), components.next()),
        (Some(Component::Normal(name)), None) if name.to_str() == Some(value)
    );
    if one_name && value.len() > suffix.len() && value.ends_with(suffix) {
        Ok(value.to_owned())
    } else {
        Err(format!(
            "must be a file name ending in {suffix}, without a directory"
        ))
    }
}

pub fn sdk(args: SdkArgs) -> Result<(), Failure> {
    let names = args.names.into_sdk_names()?;
    let descriptors = read_descriptors(&args.descriptors)?;
    let written = write_host_sdk_files_with_names(&args.output, &descriptors, &names)?;

    println!("types: {}", written.types_path.display());
    println!("sdk: {}", written.sdk_path.display());
    Ok(())
}
