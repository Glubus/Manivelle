//! `manivelle`: the command line tool of Manivelle.
//!
//! - `manivelle init --from <program>` asks a program that opted in (it calls
//!   `Engine::setup_protocol`) for its scripting API, and sets up a script project
//!   around the TypeScript declarations it gets.
//! - `manivelle run` starts the target named by a package's `manivelle.kdl`, with its packages or
//!   without them (`--no-packages`), and stays connected to it.
//! - `manivelle sdk <descriptors.json> <dir>` writes the declarations and SDK from a
//!   descriptors file.

#[path = "manivelle/check.rs"]
mod check;
#[path = "manivelle/run.rs"]
mod run;

use std::error::Error;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::path::{Component, Path, PathBuf};
use std::process::{Child, Command as Process, ExitCode, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use clap::{Args, Parser, Subcommand};
use manivelle::{
    DESCRIPTORS_FILE, EXPORT_SDK_ENV, HostContractDescriptor, MANIFEST_FILE, SdkFileNames,
    write_host_sdk_files_with_names,
};

type Failure = Box<dyn Error>;

/// Where `init` puts the files the program generated; they are rewritten on every run.
const GENERATED_DIR: &str = "manivelle";

const MAIN_TS: &str = r#"import { console } from "manivelle:env";

export function hello(): void {
  console.log("hello from a Manivelle script");
}
"#;

#[derive(Parser)]
#[command(
    name = "manivelle",
    version,
    about = "Tools for Manivelle script projects"
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Set up a script project from a program that opted in to exporting its API
    ///
    /// Runs the program (which must call `Engine::setup_protocol`) and writes
    /// its TypeScript declarations to DIR/manivelle/, plus a tsconfig.json and
    /// src/main.ts when they are missing. Run it again to refresh the declarations.
    /// Running a program means trusting it: only use `--from` on programs you would run
    /// anyway.
    Init(InitArgs),
    /// Start the target of a package, with its packages or without, and stay connected to it
    ///
    /// Reads the `manivelle.kdl` of DIRECTORY, starts its `target` and gives it the package
    /// (plus any `--packages`) to load: the target must call `Engine::setup_protocol`.
    /// `--no-packages` starts it vanilla. Running a program means trusting it.
    Run(run::RunArgs),
    /// Check packages without loading them into a program
    ///
    /// Reads every `manivelle.kdl` (DIRECTORY and any `--packages`) and checks the set:
    /// unique names, engine and dependency versions, cycles, entry files. With `--from
    /// <program>` or `--descriptors <file>` it also checks each permission against the
    /// program's host functions and events, and suggests the closest name for a typo.
    /// `--from` starts the program once, which means trusting it.
    Check(check::CheckArgs),
    /// Write the declarations and SDK module for a descriptors file
    Sdk(SdkArgs),
}

#[derive(Args)]
struct InitArgs {
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

#[derive(Args)]
struct SdkArgs {
    /// The descriptors file
    descriptors: PathBuf,
    /// Where to write the files
    output: PathBuf,
    #[command(flatten)]
    names: FileNames,
}

/// The names of the generated files, plain file names that cannot leave their directory.
#[derive(Args)]
struct FileNames {
    /// Declaration file name
    #[arg(long, default_value = "manivelle.d.ts", value_parser = declaration_name)]
    types: String,
    /// SDK module file name
    #[arg(long, default_value = "manivelle.sdk.ts", value_parser = module_name)]
    sdk: String,
}

impl FileNames {
    fn into_sdk_names(self) -> Result<SdkFileNames, Failure> {
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

fn main() -> ExitCode {
    match run(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn run(cli: Cli) -> Result<(), Failure> {
    match cli.command {
        Command::Init(args) => init(args),
        Command::Run(args) => run::run_target(args),
        Command::Check(args) => check::check(args),
        Command::Sdk(args) => sdk(args),
    }
}

fn init(args: InitArgs) -> Result<(), Failure> {
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

/// Starts the program with the export request in its environment, and waits for it until
/// `timeout`; it must exit successfully having written its descriptors to `export_dir`.
fn run_program(
    program: &Path,
    program_args: &[String],
    export_dir: &Path,
    timeout: Duration,
) -> Result<(), Failure> {
    println!("running {}", program.display());
    let mut child = Process::new(program)
        .args(program_args)
        .env(EXPORT_SDK_ENV, export_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    let Some(status) = wait_or_kill(&mut child, timeout)? else {
        return Err(format!(
            "{} did not export its SDK within {} s: it must call \
             `Engine::setup_protocol` at startup and exit when `exported` is set",
            program.display(),
            timeout.as_secs()
        )
        .into());
    };
    check_exported(program, status, export_dir)
}

/// The exit status of `child`, or `None` once `timeout` passed and it was killed.
fn wait_or_kill(child: &mut Child, timeout: Duration) -> io::Result<Option<ExitStatus>> {
    let started = Instant::now();
    loop {
        if let Some(status) = child.try_wait()? {
            return Ok(Some(status));
        }
        if started.elapsed() >= timeout {
            let _ = child.kill();
            let _ = child.wait();
            return Ok(None);
        }
        std::thread::sleep(Duration::from_millis(20));
    }
}

/// Whether the program that exited with `status` left its descriptors in `export_dir`.
fn check_exported(program: &Path, status: ExitStatus, export_dir: &Path) -> Result<(), Failure> {
    if !status.success() {
        return Err(format!(
            "{} exited with {status}; run it with {EXPORT_SDK_ENV} set to a directory to see why",
            program.display()
        )
        .into());
    }
    if !export_dir.join(DESCRIPTORS_FILE).is_file() {
        return Err(format!(
            "{} exited without exporting its SDK: it must call \
             `Engine::setup_protocol` at startup",
            program.display()
        )
        .into());
    }
    Ok(())
}

fn read_descriptors(path: &Path) -> Result<Vec<HostContractDescriptor>, Failure> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
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
    for contract in check::callable_names(descriptors) {
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

/// Writes `content` to `path`, replacing what is there. The file is written beside it and
/// renamed over it, so a symbolic link at `path` is replaced rather than followed to
/// whatever it points at.
fn replace_file(path: &Path, content: &[u8]) -> Result<(), Failure> {
    let directory = path.parent().unwrap_or_else(|| Path::new("."));
    let mut file = tempfile::NamedTempFile::new_in(directory)?;
    file.write_all(content)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.as_file()
            .set_permissions(fs::Permissions::from_mode(0o644))?;
    }
    file.persist(path).map_err(|error| error.error)?;
    Ok(())
}

/// Creates `path` with `content` unless something is there already, a dangling symbolic
/// link included (`create_new` never follows one).
fn create_if_missing(path: &Path, content: &str) -> Result<(), Failure> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(mut file) => {
            file.write_all(content.as_bytes())?;
            println!("created {}", path.display());
            Ok(())
        }
        Err(error) if error.kind() == io::ErrorKind::AlreadyExists => Ok(()),
        Err(error) => Err(error.into()),
    }
}

fn sdk(args: SdkArgs) -> Result<(), Failure> {
    let names = args.names.into_sdk_names()?;
    let descriptors = read_descriptors(&args.descriptors)?;
    let written = write_host_sdk_files_with_names(&args.output, &descriptors, &names)?;

    println!("types: {}", written.types_path.display());
    println!("sdk: {}", written.sdk_path.display());
    Ok(())
}
