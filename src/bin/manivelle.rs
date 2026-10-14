//! `manivelle`: the command line tool of Manivelle.
//!
//! - `manivelle init --from <program>` asks a program that opted in (it calls
//!   `manivelle::export_sdk_if_requested`) for its scripting API, and sets up a script
//!   project around the TypeScript declarations it gets.
//! - `manivelle sdk <descriptors.json> <dir>` writes the declarations and SDK from a
//!   descriptors file.

use std::env;
use std::error::Error;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::{Duration, Instant};

use manivelle::{
    DESCRIPTORS_FILE, EXPORT_SDK_ENV, HostContractDescriptor, SdkFileNames,
    write_host_sdk_files_with_names,
};

type Failure = Box<dyn Error>;

/// Where `init` puts the files the program generated; they are rewritten on every run.
const GENERATED_DIR: &str = "manivelle";

/// How long `init` waits for the program to export before it gives up.
const DEFAULT_TIMEOUT: Duration = Duration::from_secs(30);

const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "target": "ES2022",
    "module": "ES2022",
    "moduleResolution": "Bundler",
    "lib": ["ES2023", "DOM"],
    "strict": true,
    "noEmit": true,
    "skipLibCheck": true
  },
  "include": ["src", "manivelle/*.d.ts"]
}
"#;

const MAIN_TS: &str = r#"import { console } from "manivelle:env";

export function hello(): void {
  console.log("hello from a Manivelle script");
}
"#;

fn main() -> ExitCode {
    match run(env::args().skip(1).collect()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(args: Vec<String>) -> Result<(), Failure> {
    let mut args = args.into_iter();
    match args.next().as_deref() {
        Some("init") => init(InitArgs::parse(args)?),
        Some("sdk") => sdk(SdkArgs::parse(args)?),
        Some("--help" | "-h" | "help") | None => {
            println!("{}", usage());
            Ok(())
        }
        Some(other) => Err(format!("unknown command `{other}`\n{}", usage()).into()),
    }
}

fn usage() -> &'static str {
    "usage:
  manivelle init [DIR] --from <program> [--timeout <seconds>] [--types <name>] [--sdk <name>] [-- <program args>...]
      Runs <program> (which must call `manivelle::export_sdk_if_requested`) and writes its
      TypeScript declarations to DIR/manivelle/, plus a tsconfig.json and src/main.ts when
      they are missing. Run it again to update the declarations.
  manivelle sdk <descriptors.json> <output-dir> [--types <name>] [--sdk <name>]
      Writes the declarations and SDK module for a descriptors file."
}

/// A flag's value, from `--flag=value` or from the next argument.
fn flag_value(
    flag: &str,
    inline: Option<&str>,
    rest: &mut impl Iterator<Item = String>,
) -> Result<String, Failure> {
    match inline {
        Some(value) => Ok(value.to_owned()),
        None => rest
            .next()
            .ok_or_else(|| format!("missing value for {flag}").into()),
    }
}

struct InitArgs {
    directory: PathBuf,
    program: PathBuf,
    program_args: Vec<String>,
    timeout: Duration,
    names: SdkFileNames,
}

impl InitArgs {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, Failure> {
        let mut args = args;
        let mut directory = None;
        let mut program = None;
        let mut program_args = Vec::new();
        let mut timeout = DEFAULT_TIMEOUT;
        let mut names = SdkFileNames::default();
        while let Some(argument) = args.next() {
            let (flag, inline) = match argument.split_once('=') {
                Some((flag, value)) if flag.starts_with("--") => (flag.to_owned(), Some(value)),
                _ => (argument.clone(), None),
            };
            match flag.as_str() {
                "--" => {
                    program_args = args.by_ref().collect();
                }
                "--from" => program = Some(PathBuf::from(flag_value(&flag, inline, &mut args)?)),
                "--timeout" => {
                    let seconds: u64 = flag_value(&flag, inline, &mut args)?
                        .parse()
                        .map_err(|_| "--timeout takes a number of seconds")?;
                    timeout = Duration::from_secs(seconds);
                }
                "--types" => names.types = flag_value(&flag, inline, &mut args)?,
                "--sdk" => names.sdk = flag_value(&flag, inline, &mut args)?,
                other if other.starts_with('-') => {
                    return Err(format!("unknown argument `{other}`\n{}", usage()).into());
                }
                _ if directory.is_none() => directory = Some(PathBuf::from(argument)),
                _ => return Err(format!("unexpected argument `{argument}`\n{}", usage()).into()),
            }
        }
        Ok(Self {
            directory: directory.unwrap_or_else(|| PathBuf::from(".")),
            program: program.ok_or_else(|| format!("missing --from <program>\n{}", usage()))?,
            program_args,
            timeout,
            names,
        })
    }
}

fn init(args: InitArgs) -> Result<(), Failure> {
    let exported = tempfile::tempdir()?;
    run_program(&args, exported.path())?;
    let descriptors: Vec<HostContractDescriptor> =
        serde_json::from_str(&fs::read_to_string(exported.path().join(DESCRIPTORS_FILE))?)?;

    let generated = args.directory.join(GENERATED_DIR);
    fs::create_dir_all(&generated)?;
    let defaults = SdkFileNames::default();
    for (from, to) in [
        (&defaults.types, &args.names.types),
        (&defaults.sdk, &args.names.sdk),
    ] {
        fs::copy(exported.path().join(from), generated.join(to))?;
    }
    println!(
        "{} contract(s) from {}: wrote {} and {} in {}",
        descriptors.len(),
        args.program.display(),
        args.names.types,
        args.names.sdk,
        generated.display()
    );

    let mut tsconfig = TSCONFIG.to_owned();
    if args.names.types != defaults.types {
        tsconfig = tsconfig.replace(
            "manivelle/*.d.ts",
            &format!("manivelle/{}", args.names.types),
        );
    }
    write_if_missing(&args.directory.join("tsconfig.json"), &tsconfig)?;
    write_if_missing(&args.directory.join("src").join("main.ts"), MAIN_TS)?;
    Ok(())
}

/// Starts the program with the export request in its environment and waits for it,
/// until `args.timeout`.
fn run_program(args: &InitArgs, directory: &Path) -> Result<(), Failure> {
    let mut child = Command::new(&args.program)
        .args(&args.program_args)
        .env(EXPORT_SDK_ENV, directory)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", args.program.display()))?;
    let started = Instant::now();
    let status = loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if started.elapsed() >= args.timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Err(format!(
                "{} did not export its SDK within {} s: it must call \
                 `manivelle::export_sdk_if_requested` at startup and exit when it returns true",
                args.program.display(),
                args.timeout.as_secs()
            )
            .into());
        }
        std::thread::sleep(Duration::from_millis(20));
    };
    if !status.success() {
        return Err(format!(
            "{} exited with {status}; run it with {EXPORT_SDK_ENV} set to a directory to see why",
            args.program.display()
        )
        .into());
    }
    if !directory.join(DESCRIPTORS_FILE).is_file() {
        return Err(format!(
            "{} exited without exporting its SDK: it must call \
             `manivelle::export_sdk_if_requested` at startup",
            args.program.display()
        )
        .into());
    }
    Ok(())
}

fn write_if_missing(path: &Path, content: &str) -> Result<(), Failure> {
    if path.exists() {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    fs::write(path, content)?;
    println!("created {}", path.display());
    Ok(())
}

struct SdkArgs {
    descriptors_path: PathBuf,
    output_dir: PathBuf,
    names: SdkFileNames,
}

impl SdkArgs {
    fn parse(args: impl Iterator<Item = String>) -> Result<Self, Failure> {
        let mut args = args;
        let descriptors_path = args
            .next()
            .ok_or_else(|| format!("missing descriptors json path\n{}", usage()))?;
        let output_dir = args
            .next()
            .ok_or_else(|| format!("missing output directory\n{}", usage()))?;
        let mut names = SdkFileNames::default();
        while let Some(argument) = args.next() {
            let (flag, inline) = match argument.split_once('=') {
                Some((flag, value)) => (flag.to_owned(), Some(value)),
                None => (argument.clone(), None),
            };
            match flag.as_str() {
                "--types" => names.types = flag_value(&flag, inline, &mut args)?,
                "--sdk" => names.sdk = flag_value(&flag, inline, &mut args)?,
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

fn sdk(args: SdkArgs) -> Result<(), Failure> {
    let source = fs::read_to_string(&args.descriptors_path)?;
    let descriptors: Vec<HostContractDescriptor> = serde_json::from_str(&source)?;
    let written = write_host_sdk_files_with_names(&args.output_dir, &descriptors, &args.names)?;

    println!("types: {}", written.types_path.display());
    println!("sdk: {}", written.sdk_path.display());
    Ok(())
}
