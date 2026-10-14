//! `manivelle-sdk`: writes the TypeScript declarations and SDK for host contract descriptors.

use std::env;
use std::error::Error;
use std::path::PathBuf;
use std::process::ExitCode;

use manivelle::{HostContractDescriptor, SdkFileNames, write_host_sdk_files_with_names};

fn main() -> ExitCode {
    let args: Vec<String> = env::args().skip(1).collect();
    if args.iter().any(|arg| arg == "--help" || arg == "-h") {
        println!("{}", usage());
        return ExitCode::SUCCESS;
    }

    match run(args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), Box<dyn Error>> {
    let args = ExportArgs::parse(args)?;
    let descriptors = read_descriptors(&args.descriptors_path)?;
    let written = write_host_sdk_files_with_names(&args.output_dir, &descriptors, &args.names)?;

    println!("types: {}", written.types_path.display());
    println!("sdk: {}", written.sdk_path.display());
    Ok(())
}

struct ExportArgs {
    descriptors_path: PathBuf,
    output_dir: PathBuf,
    names: SdkFileNames,
}

impl ExportArgs {
    fn parse(args: impl IntoIterator<Item = String>) -> Result<Self, Box<dyn Error>> {
        let mut args = args.into_iter();
        let descriptors_path = required_arg(args.next(), "missing descriptors json path")?;
        let output_dir = required_arg(args.next(), "missing output directory")?;
        let mut names = SdkFileNames::default();

        while let Some(flag) = args.next() {
            match flag.as_str() {
                "--types" => names.types = required_arg(args.next(), "missing --types value")?,
                "--sdk" => names.sdk = required_arg(args.next(), "missing --sdk value")?,
                other => return Err(format!("unknown argument `{other}`\n{}", usage()).into()),
            }
        }

        Ok(Self {
            descriptors_path: PathBuf::from(descriptors_path),
            output_dir: PathBuf::from(output_dir),
            names,
        })
    }
}

fn read_descriptors(path: &PathBuf) -> Result<Vec<HostContractDescriptor>, Box<dyn Error>> {
    let source = std::fs::read_to_string(path)?;
    Ok(serde_json::from_str(&source)?)
}

fn required_arg(value: Option<String>, message: &str) -> Result<String, Box<dyn Error>> {
    value.ok_or_else(|| format!("{message}\n{}", usage()).into())
}

fn usage() -> &'static str {
    "usage: manivelle-sdk <descriptors.json> <output-dir> [--types manivelle.d.ts] [--sdk manivelle.sdk.ts]"
}
