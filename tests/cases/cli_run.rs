//! `manivelle run`: the tool starts a target with its packages, or without them, and stays
//! connected to it.
//!
//! The target is this very test executable, started with a filter that selects
//! `run_child`, which calls `Engine::setup_protocol` the way a real program does.

use crate::support;

use std::fs;
use std::io::{BufRead, BufReader, Read, Write};
use std::path::Path;
use std::process::{Command, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use manivelle::{Engine, EngineOptions};
use support::{TestCacheDir, stderr, stdout};

/// What the target writes once its packages are loaded and it polls for a reload.
const READY_LINE: &str = "run_child ready";

/// The program `manivelle run` starts. Run on its own, it does nothing.
#[test]
fn run_child() {
    let Some(out) = std::env::var_os("RUN_TEST_OUT") else {
        return;
    };
    let out = Path::new(&out);
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    let setup = engine.setup_protocol().expect("set up the protocol");
    let names: Vec<&str> = setup
        .packages
        .loaded
        .iter()
        .map(|m| m.name.as_str())
        .collect();
    let mut report = format!("attached={} packages={}\n", setup.attached, names.join(","));
    if std::env::var_os("RUN_TEST_WAIT_RELOAD").is_some() {
        // The tool passes the target's stdout through: this line tells the test that
        // the package is loaded and the target polls for the reload.
        println!("{READY_LINE}");
        std::io::stdout().flush().expect("flush stdout");
        let deadline = Instant::now() + Duration::from_secs(10);
        let mut value: f64 = engine.call("hello", "v", ()).expect("call v");
        while value != 22.0 && Instant::now() < deadline {
            engine.poll_protocol();
            sleep(Duration::from_millis(20));
            value = engine.call("hello", "v", ()).expect("call v");
        }
        report += &format!("v={value}\n");
    }
    if std::env::var_os("RUN_TEST_LOG").is_some() {
        // No pause before the process ends: dropping the engine flushes the line.
        engine.call::<()>("hello", "say", ()).expect("call say");
    }
    fs::write(out.join("report"), report).expect("write report");
    if let Some(code) = std::env::var("RUN_TEST_EXIT")
        .ok()
        .and_then(|code| code.parse().ok())
    {
        std::process::exit(code);
    }
}

fn project(dir: &Path, target: Option<&Path>) {
    // A raw string: a Windows path has backslashes, which a quoted KDL string reads as escapes.
    let target = target.map_or(String::new(), |t| format!("target #\"{}\"#\n", t.display()));
    fs::write(
        dir.join("manivelle.kdl"),
        format!("package \"hello\" {{\n version \"1.0.0\"\n entry \"main.ts\"\n {target}}}\n"),
    )
    .expect("manifest");
    fs::write(
        dir.join("main.ts"),
        "export function v(): number { return 1; }\n\
         export function say(): void { console.log(\"hi from hello\"); console.error(\"oops\"); }",
    )
    .expect("script");
}

#[test]
fn run_shows_the_console_of_the_scripts_once() {
    let dir = TestCacheDir::new("cli-run-console");
    let exe = std::env::current_exe().expect("test executable");
    project(dir.path(), Some(&exe));

    let output = run_command(dir.path(), dir.path(), &[])
        .env("RUN_TEST_LOG", "1")
        .output()
        .expect("run manivelle");

    assert!(output.status.success(), "{}", stderr(&output));
    // Once: the tool shows it, and the program's own stderr writer stays silent.
    assert_eq!(
        stdout(&output)
            .matches("[log] hello: hi from hello")
            .count(),
        1,
        "{}",
        stdout(&output)
    );
    assert_eq!(
        stderr(&output).matches("[error] hello: oops").count(),
        1,
        "{}",
        stderr(&output)
    );
}

fn run_command(dir: &Path, out: &Path, extra: &[&str]) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_manivelle"));
    command
        .arg("run")
        .arg(dir)
        .args(extra)
        .args(["--", "--exact", "cases::cli_run::run_child", "--nocapture"])
        .env("RUN_TEST_OUT", out);
    command
}

fn report(out: &Path) -> String {
    fs::read_to_string(out.join("report")).expect("the target wrote its report")
}

#[test]
fn run_starts_the_target_with_its_packages() {
    let dir = TestCacheDir::new("cli-run");
    let exe = std::env::current_exe().expect("test executable");
    project(dir.path(), Some(&exe));

    let output = run_command(dir.path(), dir.path(), &[])
        .output()
        .expect("run manivelle");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(report(dir.path()), "attached=true packages=hello\n");
    assert!(
        stdout(&output).contains("package hello 1.0.0 loaded"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn run_without_packages_starts_the_target_vanilla() {
    let dir = TestCacheDir::new("cli-run-vanilla");
    let exe = std::env::current_exe().expect("test executable");
    project(dir.path(), Some(&exe));

    let output = run_command(dir.path(), dir.path(), &["--no-packages"])
        .output()
        .expect("run manivelle");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(report(dir.path()), "attached=true packages=\n");
    assert!(!stdout(&output).contains("loaded"), "{}", stdout(&output));
}

#[test]
fn run_needs_a_target() {
    let dir = TestCacheDir::new("cli-run-no-target");
    project(dir.path(), None);

    let output = run_command(dir.path(), dir.path(), &[])
        .output()
        .expect("run manivelle");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("no target to run"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn run_without_packages_needs_no_manifest_when_given_a_target() {
    let dir = TestCacheDir::new("cli-run-no-manifest");
    let exe = std::env::current_exe().expect("test executable");

    let output = run_command(
        dir.path(),
        dir.path(),
        &[
            "--no-packages",
            "--target",
            exe.to_str().expect("utf-8 path"),
        ],
    )
    .output()
    .expect("run manivelle");

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(report(dir.path()), "attached=true packages=\n");
}

#[test]
fn run_without_packages_still_reports_a_manifest_that_is_broken() {
    let dir = TestCacheDir::new("cli-run-broken-manifest");
    let exe = std::env::current_exe().expect("test executable");
    fs::write(dir.path().join("manivelle.kdl"), "package \"hello\" {\n").expect("manifest");

    let output = run_command(
        dir.path(),
        dir.path(),
        &[
            "--no-packages",
            "--target",
            exe.to_str().expect("utf-8 path"),
        ],
    )
    .output()
    .expect("run manivelle");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("manivelle.kdl"),
        "{}",
        stderr(&output)
    );
    assert!(!dir.path().join("report").exists(), "the target ran");
}

#[test]
fn run_exits_with_the_exit_code_of_the_target() {
    let dir = TestCacheDir::new("cli-run-exit-code");
    let exe = std::env::current_exe().expect("test executable");
    project(dir.path(), Some(&exe));

    let output = run_command(dir.path(), dir.path(), &[])
        .env("RUN_TEST_EXIT", "3")
        .output()
        .expect("run manivelle");

    assert_eq!(output.status.code(), Some(3), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("exited with"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn run_watch_reloads_a_mod_whose_files_changed() {
    let dir = TestCacheDir::new("cli-run-watch");
    let exe = std::env::current_exe().expect("test executable");
    project(dir.path(), Some(&exe));

    let mut child = run_command(dir.path(), dir.path(), &["--watch"])
        .env("RUN_TEST_WAIT_RELOAD", "1")
        .stdout(Stdio::piped())
        .spawn()
        .expect("run manivelle");
    let mut stdout = BufReader::new(child.stdout.take().expect("piped stdout"));
    let mut seen = String::new();
    let mut line = String::new();
    while !seen.contains(READY_LINE) && stdout.read_line(&mut line).expect("read stdout") != 0 {
        seen += &line;
        line.clear();
    }
    assert!(
        seen.contains(READY_LINE),
        "the target never got ready: {seen}"
    );
    fs::write(
        dir.path().join("main.ts"),
        "export function v(): number { return 22; }",
    )
    .expect("edit the package");
    stdout.read_to_string(&mut seen).expect("read stdout");
    let status = child.wait().expect("wait for manivelle");

    assert!(status.success(), "{seen}");
    assert!(
        report(dir.path()).contains("v=22"),
        "{}",
        report(dir.path())
    );
    assert!(seen.contains("reloaded hello"), "{seen}");
}
