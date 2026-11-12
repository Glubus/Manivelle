//! Running a program once to export its SDK: its descriptors and declarations.

use std::fs::{self, File};
use std::io::{self, Read, Seek, SeekFrom};
use std::path::Path;
use std::process::{Child, Command as Process, ExitStatus, Stdio};
use std::time::{Duration, Instant};

use manivelle::{DESCRIPTORS_FILE, EXPORT_SDK_ENV, HostContractDescriptor};

use super::Failure;

/// How many of the last lines the program wrote to stderr an error quotes.
const STDERR_TAIL_LINES: usize = 20;

/// Starts the program with the export request in its environment, and waits for it until
/// `timeout`; it must exit successfully having written its descriptors to `export_dir`.
/// What it writes to stderr is kept out of the way, in a file rather than a pipe that a
/// chatty program would fill, and quoted when the export fails.
pub(super) fn run_program(
    program: &Path,
    program_args: &[String],
    export_dir: &Path,
    timeout: Duration,
) -> Result<(), Failure> {
    println!("running {}", program.display());
    let mut stderr = tempfile::tempfile()?;
    let mut child = Process::new(program)
        .args(program_args)
        .env(EXPORT_SDK_ENV, export_dir)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(stderr.try_clone()?)
        .spawn()
        .map_err(|error| format!("cannot start {}: {error}", program.display()))?;
    let outcome = match wait_or_kill(&mut child, timeout)? {
        Some(status) => check_exported(program, status, export_dir),
        None => Err(format!(
            "{} did not export its SDK within {} s: it must call \
             `Engine::setup_protocol` at startup and exit when `exported` is set",
            program.display(),
            timeout.as_secs()
        )),
    };
    outcome.map_err(|failure| format!("{failure}{}", stderr_tail(&mut stderr)).into())
}

/// The last lines the program wrote to `stderr`, introduced for the end of an error
/// message; nothing when it wrote none or they cannot be read.
fn stderr_tail(stderr: &mut File) -> String {
    let mut text = String::new();
    let read = stderr
        .seek(SeekFrom::Start(0))
        .and_then(|_| stderr.read_to_string(&mut text));
    let lines: Vec<&str> = text.lines().collect();
    if read.is_err() || lines.is_empty() {
        return String::new();
    }
    let tail = &lines[lines.len().saturating_sub(STDERR_TAIL_LINES)..];
    format!("\nits stderr ended with:\n{}", tail.join("\n"))
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
fn check_exported(program: &Path, status: ExitStatus, export_dir: &Path) -> Result<(), String> {
    if !status.success() {
        return Err(format!("{} exited with {status}", program.display()));
    }
    if !export_dir.join(DESCRIPTORS_FILE).is_file() {
        return Err(format!(
            "{} exited without exporting its SDK: it must call \
             `Engine::setup_protocol` at startup",
            program.display()
        ));
    }
    Ok(())
}

pub(super) fn read_descriptors(path: &Path) -> Result<Vec<HostContractDescriptor>, Failure> {
    Ok(serde_json::from_str(&fs::read_to_string(path)?)?)
}
