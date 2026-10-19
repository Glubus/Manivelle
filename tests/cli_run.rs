#![cfg(feature = "cli")]

//! `manivelle run`: the tool starts a target with its packages, or without them, and stays
//! connected to it.
//!
//! The target is this very test executable, started with a filter that selects
//! `run_child`, which calls `Engine::setup_protocol` the way a real program does.

mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output, Stdio};
use std::thread::sleep;
use std::time::{Duration, Instant};

use manivelle::{Engine, EngineOptions};
use support::TestCacheDir;

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
        fs::write(out.join("ready"), "").expect("signal readiness");
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
        engine.call::<()>("hello", "say", ()).expect("call say");
        // The line leaves on a thread of its own: give it time before the process ends.
        sleep(Duration::from_millis(300));
    }
    fs::write(out.join("report"), report).expect("write report");
}

fn project(dir: &Path, target: Option<&Path>) {
    let target = target.map_or(String::new(), |t| format!("target \"{}\"\n", t.display()));
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
        .args(["--", "--exact", "run_child", "--nocapture"])
        .env("RUN_TEST_OUT", out);
    command
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
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
fn run_watch_reloads_a_mod_whose_files_changed() {
    let dir = TestCacheDir::new("cli-run-watch");
    let exe = std::env::current_exe().expect("test executable");
    project(dir.path(), Some(&exe));

    let child = run_command(dir.path(), dir.path(), &["--watch"])
        .env("RUN_TEST_WAIT_RELOAD", "1")
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("run manivelle");
    let started = Instant::now();
    while !dir.path().join("ready").exists() {
        assert!(
            started.elapsed() < Duration::from_secs(20),
            "the target never got ready"
        );
        sleep(Duration::from_millis(20));
    }
    fs::write(
        dir.path().join("main.ts"),
        "export function v(): number { return 22; }",
    )
    .expect("edit the package");
    let output = child.wait_with_output().expect("wait for manivelle");

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        report(dir.path()).contains("v=22"),
        "{}",
        report(dir.path())
    );
    assert!(
        stdout(&output).contains("reloaded hello"),
        "{}",
        stdout(&output)
    );
}
