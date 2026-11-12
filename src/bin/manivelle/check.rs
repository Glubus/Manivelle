//! `manivelle check`: validates packages without loading them into a program.

use std::fs;
use std::iter;
use std::path::PathBuf;
use std::time::Duration;

use clap::Args;
use manivelle::{
    HostContractDescriptor, HostContractKind, MANIFEST_FILE, PackageManifest, PackageReport,
    SdkFileNames, Severity, add_permissions, check_package, resolve_packages,
};

use super::Failure;
use super::export::{read_descriptors, run_program};
use super::files::replace_file;
use super::init::GENERATED_DIR;
use super::roots::canonical_roots;

#[derive(Args)]
pub struct CheckArgs {
    /// The package's directory, with its `manivelle.kdl`
    #[arg(default_value = ".")]
    directory: PathBuf,
    /// Also check the packages in DIR (a package, or a directory of packages)
    #[arg(long = "packages", value_name = "DIR")]
    package_dirs: Vec<PathBuf>,
    /// Compare the packages with the contracts of this program, which is started once to
    /// export them; it must call `Engine::setup_protocol`
    #[arg(long, conflicts_with = "descriptors")]
    from: Option<PathBuf>,
    /// Compare the packages with the contracts of this descriptors file, as
    /// `engine.registry().descriptors()` serialized as JSON
    #[arg(long)]
    descriptors: Option<PathBuf>,
    /// Add to the manifest of DIRECTORY the permissions its code uses and it does not list
    /// (never removes one); needs `--from` or `--descriptors`
    #[arg(long)]
    fix: bool,
    /// Seconds to wait for `--from` to export its contracts
    #[arg(long, default_value_t = 30)]
    timeout: u64,
    /// Arguments for `--from`, after `--`
    #[arg(last = true)]
    program_args: Vec<String>,
}

pub fn check(args: CheckArgs) -> Result<(), Failure> {
    let directory = fs::canonicalize(&args.directory)
        .map_err(|error| format!("cannot find {}: {error}", args.directory.display()))?;
    let roots =
        canonical_roots(iter::once(directory.clone()).chain(args.package_dirs.iter().cloned()))?;
    let packages = resolve_packages(&roots)?;
    let (descriptors, declarations) = contracts(&args)?;
    if args.fix && descriptors.is_none() {
        return Err("--fix needs --from <program> or --descriptors <file>".into());
    }

    let mut errors = 0;
    for package in &packages {
        let mut report = report_of(package, descriptors.as_deref())?;
        if args.fix && package.root == directory && !report.missing_permissions.is_empty() {
            report = fix(package, descriptors.as_deref().unwrap_or_default(), &report)?;
        }
        if let Some(declarations) = &declarations {
            report
                .findings
                .extend(stale_declarations(package, declarations));
        }
        errors += print(package, &report);
    }
    if descriptors.is_none() {
        println!(
            "[manivelle] the code and the permissions were not compared with a program: pass \
             --from <program> or --descriptors <file>"
        );
    }
    if errors == 0 {
        Ok(())
    } else {
        Err(format!("{errors} error(s) found").into())
    }
}

/// The report of one package: all of [`check_package`] when the contracts are known, and
/// only the entry file otherwise.
fn report_of(
    package: &PackageManifest,
    descriptors: Option<&[HostContractDescriptor]>,
) -> Result<PackageReport, Failure> {
    if let Some(descriptors) = descriptors {
        return Ok(check_package(package, descriptors)?);
    }
    let mut report = PackageReport::default();
    let entry = package.entry_path();
    if !entry.is_file() {
        // `check_package` words it the same way; with no contracts it cannot run.
        report.findings.push(manivelle::Finding {
            severity: Severity::Error,
            message: format!("the entry {} is not a file", entry.display()),
        });
    }
    Ok(report)
}

/// Adds the missing permissions to the manifest of `package` and checks it again.
fn fix(
    package: &PackageManifest,
    descriptors: &[HostContractDescriptor],
    report: &PackageReport,
) -> Result<PackageReport, Failure> {
    let path = package.root.join(MANIFEST_FILE);
    let source = fs::read_to_string(&path)?;
    replace_file(
        &path,
        add_permissions(&source, &report.missing_permissions)?.as_bytes(),
    )?;
    println!(
        "[manivelle] added to the permissions of {}: {}",
        path.display(),
        report.missing_permissions.join(", ")
    );
    let package = PackageManifest::read(&package.root)?;
    Ok(check_package(&package, descriptors)?)
}

/// Prints what was found about one package; returns how many errors it holds.
fn print(package: &PackageManifest, report: &PackageReport) -> usize {
    for finding in &report.findings {
        eprintln!("[manivelle] package {}: {finding}", package.name);
    }
    let errors = report.errors();
    if errors == 0 {
        let warnings = report.findings.len();
        let suffix = if warnings == 0 {
            String::new()
        } else {
            format!(" ({warnings} warning(s))")
        };
        println!(
            "[manivelle] package {} {}: ok{suffix}",
            package.name, package.version
        );
    }
    errors
}

/// An error when the declarations `manivelle init` wrote in `package` are no longer the
/// ones the program exports: the scripts would be type-checked against an API the program
/// does not have anymore. A package without that file (it was not set up by `init`, or
/// names its files otherwise) is not compared.
fn stale_declarations(package: &PackageManifest, exported: &str) -> Option<manivelle::Finding> {
    let path = package
        .root
        .join(GENERATED_DIR)
        .join(SdkFileNames::default().types);
    let written = fs::read_to_string(&path).ok()?;
    // A checkout may have turned the line ends of the file into CRLF.
    (written.replace("\r\n", "\n") != exported.replace("\r\n", "\n")).then(|| manivelle::Finding {
        severity: Severity::Error,
        message: format!(
            "{} is not what the program exports now: run `manivelle init --from <program>` \
             in the package to refresh it",
            path.display()
        ),
    })
}

/// The contracts of the program, when `--from` or `--descriptors` says where to read them,
/// and the declarations it exported with them when it was run.
type Contracts = (Option<Vec<HostContractDescriptor>>, Option<String>);

fn contracts(args: &CheckArgs) -> Result<Contracts, Failure> {
    Ok(match (&args.from, &args.descriptors) {
        (Some(program), _) => {
            let program = fs::canonicalize(program)
                .map_err(|error| format!("cannot find {}: {error}", program.display()))?;
            let exported = tempfile::tempdir()?;
            run_program(
                &program,
                &args.program_args,
                exported.path(),
                Duration::from_secs(args.timeout),
            )?;
            let descriptors = read_descriptors(&exported.path().join(manivelle::DESCRIPTORS_FILE))?;
            let declarations =
                fs::read_to_string(exported.path().join(SdkFileNames::default().types))?;
            (Some(descriptors), Some(declarations))
        }
        (None, Some(file)) => (Some(read_descriptors(file)?), None),
        (None, None) => (None, None),
    })
}

/// What a permission can name: host functions and events.
pub(super) fn callable_names(descriptors: &[HostContractDescriptor]) -> Vec<String> {
    descriptors
        .iter()
        .filter(|d| {
            matches!(
                d.kind,
                HostContractKind::Function | HostContractKind::Callback
            )
        })
        .map(|d| d.name.clone())
        .collect()
}
