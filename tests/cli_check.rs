#![cfg(feature = "cli")]

//! `manivelle check`: validates packages, and their permissions against a program's
//! contracts.
//!
//! The program is this very test executable, started with a filter that selects
//! `export_child`, which registers a contract and calls `Engine::setup_protocol`.

mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use manivelle::{Engine, EngineOptions, HostContract, HostContractKind, HostFunctionSignature};
use support::{TestCacheDir, stderr, stdout};

struct Add;

impl HostContract for Add {
    const NAME: &'static str = "math.add";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Add {
    type Input = f64;
    type Output = f64;
}

/// The program `manivelle check --from` starts.
#[test]
fn export_child() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with::<Add>(Ok)
        .expect("register");
    engine.setup_protocol().expect("set up the protocol");
}

fn package(dir: &Path, permissions: &str, with_entry: bool) {
    fs::create_dir_all(dir).expect("package directory");
    fs::write(
        dir.join("manivelle.kdl"),
        format!(
            "package \"hello\" {{\n version \"1.0.0\"\n entry \"main.ts\"\n permissions {{\n{permissions}\n }}\n}}\n"
        ),
    )
    .expect("manifest");
    if with_entry {
        fs::write(dir.join("main.ts"), "export const v = 1;").expect("entry");
    }
}

fn check(dir: &Path, extra: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_manivelle"))
        .arg("check")
        .arg(dir)
        .args(extra)
        .output()
        .expect("run manivelle")
}

fn check_from_program(dir: &Path) -> Output {
    let exe = std::env::current_exe().expect("test executable");
    check(
        dir,
        &[
            "--from",
            exe.to_str().expect("utf-8 path"),
            "--",
            "--exact",
            "export_child",
            "--nocapture",
        ],
    )
}

#[test]
fn a_package_whose_permissions_exist_is_ok() {
    let dir = TestCacheDir::new("cli-check-ok");
    package(dir.path(), "  math.add", true);

    let output = check_from_program(dir.path());

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("package hello 1.0.0: ok"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_misspelled_permission_is_flagged_with_the_closest_contract() {
    let dir = TestCacheDir::new("cli-check-typo");
    package(dir.path(), "  math.ad", true);

    let output = check_from_program(dir.path());

    assert!(!output.status.success());
    let errors = stderr(&output);
    assert!(
        errors.contains("`math.ad` matches no contract")
            && errors.contains("did you mean `math.add`?"),
        "{errors}"
    );
}

#[test]
fn a_wildcard_that_covers_nothing_is_flagged() {
    let dir = TestCacheDir::new("cli-check-wildcard");
    package(dir.path(), "  physics.*", true);

    let output = check_from_program(dir.path());

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("`physics.*` matches no contract"),
        "{}",
        stderr(&output)
    );
}

#[test]
fn a_descriptors_file_stands_in_for_the_program() {
    let dir = TestCacheDir::new("cli-check-descriptors");
    package(dir.path(), "  math.add", true);
    let descriptors = dir.path().join("descriptors.json");
    fs::write(
        &descriptors,
        serde_json::to_string(&[Add::descriptor()]).expect("descriptors"),
    )
    .expect("write descriptors");

    let output = check(
        dir.path(),
        &["--descriptors", descriptors.to_str().expect("utf-8")],
    );

    assert!(output.status.success(), "{}", stderr(&output));
}

#[test]
fn without_a_program_only_the_packages_are_checked() {
    let dir = TestCacheDir::new("cli-check-offline");
    package(dir.path(), "  anything.goes", true);

    let output = check(dir.path(), &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stdout(&output).contains("were not compared with a program"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn a_missing_entry_file_and_a_missing_dependency_are_flagged() {
    let dir = TestCacheDir::new("cli-check-entry");
    package(dir.path(), "", false);
    let output = check(dir.path(), &[]);
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("is not a file"),
        "{}",
        stderr(&output)
    );

    let needy = TestCacheDir::new("cli-check-dependency");
    fs::write(
        needy.path().join("manivelle.kdl"),
        "package \"needy\" {\n version \"1.0.0\"\n entry \"main.ts\"\n depends {\n  ghost\n }\n}\n",
    )
    .expect("manifest");
    let output = check(needy.path(), &[]);
    assert!(!output.status.success());
    assert!(stderr(&output).contains("ghost"), "{}", stderr(&output));
}

fn package_with_code(dir: &Path, permissions: &str, main: &str) {
    package(dir, permissions, false);
    fs::write(dir.join("main.ts"), main).expect("entry");
}

#[test]
fn code_that_uses_an_unlisted_function_fails_and_fix_adds_the_permission() {
    let dir = TestCacheDir::new("cli-check-fix");
    package_with_code(dir.path(), "", "export const run = () => math.add(1);\n");

    let output = check_from_program(dir.path());
    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("main.ts:1: uses `math.add`, which its permissions do not list"),
        "{}",
        stderr(&output)
    );

    let exe = std::env::current_exe().expect("test executable");
    let fixed = check(
        dir.path(),
        &[
            "--fix",
            "--from",
            exe.to_str().expect("utf-8 path"),
            "--",
            "--exact",
            "export_child",
            "--nocapture",
        ],
    );
    assert!(fixed.status.success(), "{}", stderr(&fixed));
    assert!(
        stdout(&fixed).contains("added to the permissions"),
        "{}",
        stdout(&fixed)
    );
    let manifest = fs::read_to_string(dir.path().join("manivelle.kdl")).expect("manifest");
    assert!(manifest.contains("math.add"), "{manifest}");
    // And what it wrote is checked clean on its own.
    assert!(check_from_program(dir.path()).status.success());
}

#[test]
fn an_unused_permission_is_a_warning_and_does_not_fail() {
    let dir = TestCacheDir::new("cli-check-unused");
    package_with_code(dir.path(), "  math.add", "export const v = 1;\n");

    let output = check_from_program(dir.path());

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(
        stderr(&output).contains("which the code never uses"),
        "{}",
        stderr(&output)
    );
    assert!(
        stdout(&output).contains("ok (1 warning(s))"),
        "{}",
        stdout(&output)
    );
}

#[test]
fn fix_needs_the_contracts_of_a_program() {
    let dir = TestCacheDir::new("cli-check-fix-alone");
    package(dir.path(), "", true);

    let output = check(dir.path(), &["--fix"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("--fix needs --from"),
        "{}",
        stderr(&output)
    );
}
