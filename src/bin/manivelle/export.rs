//! Running a program once to export its SDK: its descriptors and declarations.

use std::fs;
use std::io;
use std::path::Path;
use std::process::{Child, Command as Process, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use manivelle::{DESCRIPTORS_FILE, EXPORT_SDK_ENV, HostContractDescriptor};

use super::Failure;

/// Starts the program with the export request in its environment, and waits for it until
/// `timeout`; it must exit successfully having written its descriptors to `export_dir`.
pub(super) fn run_program(
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

pub(super) fn read_descriptors(path: &Path) -> Result<Vec<HostContractDescriptor>, Failure> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}
