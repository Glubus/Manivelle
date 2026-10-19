//! `manivelle init`: sets up a script project around the declarations a program exports.

use std::fs;
use std::path::{Path, PathBuf};
use std::time::Duration;

use clap::Args;
use manivelle::{DESCRIPTORS_FILE, HostContractDescriptor, MANIFEST_FILE, SdkFileNames};

use super::Failure;
use super::check::callable_names;
use super::export::{read_descriptors, run_program};
use super::files::{create_if_missing, replace_file};
use super::sdk::FileNames;

/// Where `init` puts the files the program generated; they are rewritten on every run.
const GENERATED_DIR: &str = "manivelle";

const MAIN_TS: &str = r#"import { console } from "manivelle:env";

export function hello(): void {
  console.log("hello from a Manivelle script");
}
"#;

#[derive(Args)]
pub struct InitArgs {
    /// The project directory
    #[arg(default_value = ".")]
    directory: PathBuf,
    /// The program to export from, as a path (not looked up in PATH)
    #[arg(long)]
    from: PathBuf,
    /// Seconds to wait for the program to export before giving up
    #[arg(long, default_value_t = 30)]
    timeout: u64,
    #[command(flatten)]
    names: FileNames,
    /// Arguments for the program, after `--`
    #[arg(last = true)]
    program_args: Vec<String>,
}

pub fn init(args: InitArgs) -> Result<(), Failure> {
    let names = args.names.into_sdk_names()?;
    let program = fs::canonicalize(&args.from).map_err(|error| {
        format!(
            "cannot find {}: {error} (pass the path to the program, such as ./game)",
            args.from.display()
        )
    })?;
    let exported = tempfile::tempdir()?;
    run_program(
        &program,
        &args.program_args,
        exported.path(),
        Duration::from_secs(args.timeout),
    )?;
    let descriptors = read_descriptors(&exported.path().join(DESCRIPTORS_FILE))?;

    let generated = args.directory.join(GENERATED_DIR);
    copy_declarations(exported.path(), &generated, &names)?;
    println!(
        "{} contract(s) from {}: wrote {} and {} in {}",
        descriptors.len(),
        program.display(),
        names.types,
        names.sdk,
        generated.display()
    );
    scaffold_project(&args.directory, &names, &program, &descriptors)
}

/// Copies the declarations and SDK module the program wrote into the project.
fn copy_declarations(
    exported: &Path,
    generated: &Path,
    names: &SdkFileNames,
) -> Result<(), Failure> {
    fs::create_dir_all(generated)?;
    let defaults = SdkFileNames::default();
    for (from, to) in [(&defaults.types, &names.types), (&defaults.sdk, &names.sdk)] {
        replace_file(&generated.join(to), &fs::read(exported.join(from))?)?;
    }
    Ok(())
}

/// Creates the `tsconfig.json`, `src/main.ts` and `manivelle.kdl` a new project needs, when
/// missing.
fn scaffold_project(
    directory: &Path,
    names: &SdkFileNames,
    program: &Path,
    descriptors: &[HostContractDescriptor],
) -> Result<(), Failure> {
    let tsconfig = serde_json::to_string_pretty(&serde_json::json!({
        "compilerOptions": {
            "target": "ES2022",
            "module": "ES2022",
            "moduleResolution": "Bundler",
            "lib": ["ES2023", "DOM"],
            "strict": true,
            "noEmit": true,
            "skipLibCheck": true
        },
        "include": ["src", format!("{GENERATED_DIR}/{}", names.types)]
    }))?;
    create_if_missing(&directory.join("tsconfig.json"), &format!("{tsconfig}\n"))?;
    create_if_missing(&directory.join("src").join("main.ts"), MAIN_TS)?;
    create_if_missing(
        &directory.join(MANIFEST_FILE),
        &package_manifest(directory, program, descriptors)?,
    )
}

/// A `manivelle.kdl` for the project: named after its directory, with the program as its
/// target and the program's host functions and events listed, commented out, under
/// `permissions`.
fn package_manifest(
    directory: &Path,
    program: &Path,
    descriptors: &[HostContractDescriptor],
) -> Result<String, Failure> {
    fs::create_dir_all(directory)?;
    let directory = fs::canonicalize(directory)?;
    let name: String = directory
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || matches!(c, '_' | '-' | '.') {
                c
            } else {
                '-'
            }
        })
        .collect();
    let name = if name.trim_matches('-').is_empty() {
        "my-package".to_owned()
    } else {
        name
    };
    let target = relative_path(&directory, program)
        .to_string_lossy()
        .replace('\\', "/");
    let mut permissions = String::new();
    for contract in callable_names(descriptors) {
        permissions.push_str(&format!("        // {contract}\n"));
    }
    Ok(format!(
        "package {name:?} {{\n    version \"0.1.0\"\n    entry \"src/main.ts\"\n    target {target:?}\n\n    // The host functions and events this package may use; uncomment what it needs.\n    // `manivelle check --from <program>` flags a name the program does not have.\n    permissions {{\n{permissions}    }}\n}}\n"
    ))
}

/// `to` as a path from the directory `from`; both are absolute.
fn relative_path(from: &Path, to: &Path) -> PathBuf {
    let from: Vec<_> = from.components().collect();
    let to: Vec<_> = to.components().collect();
    let common = from.iter().zip(&to).take_while(|(a, b)| a == b).count();
    let mut relative = PathBuf::new();
    for _ in common..from.len() {
        relative.push("..");
    }
    for part in &to[common..] {
        relative.push(part);
    }
    relative
}
