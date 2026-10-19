#![cfg(all(feature = "derive", feature = "cli"))]

//! `manivelle init --from <program>`: the tool starts a program that opted in, takes its
//! scripting API and sets up a script project.
//!
//! The program is the `init_program` example, which registers a contract and calls
//! `Engine::setup_protocol`. `cargo test` builds examples into `target/<profile>/examples`,
//! next to the `deps` directory that holds this test executable.

mod support;

use std::fs;
use std::path::Path;
use std::process::{Command, Output};

use support::{TestCacheDir, example_program, stderr};

/// Runs `manivelle init` on `directory` with `extra` options, and `program_args` passed on
/// to the program.
fn init(directory: &Path, extra: &[&str], program_args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_manivelle"))
        .arg("init")
        .arg(directory)
        .arg("--from")
        .arg(example_program("init_program"))
        .args(extra)
        .arg("--")
        .args(program_args)
        .output()
        .expect("run manivelle")
}

#[test]
fn init_writes_the_declarations_of_the_program_and_a_project_around_them() {
    let dir = TestCacheDir::new("cli-init");
    let project = dir.path().join("my-mod");

    let output = init(&project, &[], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    let types = fs::read_to_string(project.join("manivelle/manivelle.d.ts")).expect("types");
    assert!(types.contains("math"), "{types}");
    assert!(project.join("manivelle/manivelle.sdk.ts").is_file());
    assert!(project.join("tsconfig.json").is_file());
    assert!(project.join("src/main.ts").is_file());

    // A package manifest named after the directory, with the program as its target and
    // its contracts listed, commented out, as the permissions to pick from.
    let manifest = fs::read_to_string(project.join("manivelle.kdl")).expect("manifest");
    assert!(manifest.contains("package \"my-mod\""), "{manifest}");
    assert!(manifest.contains("// math.add"), "{manifest}");
    let target = manifest
        .lines()
        .find_map(|line| line.trim().strip_prefix("target "))
        .expect("a target line");
    assert!(
        target.contains(
            example_program("init_program")
                .file_name()
                .and_then(|n| n.to_str())
                .expect("file name")
        ),
        "{target}"
    );
    // And it is a manifest the tool accepts.
    let checked = Command::new(env!("CARGO_BIN_EXE_manivelle"))
        .arg("check")
        .arg(&project)
        .output()
        .expect("run manivelle check");
    assert!(checked.status.success(), "{}", stderr(&checked));
}

#[test]
fn the_project_init_writes_type_checks() {
    let dir = TestCacheDir::new("cli-init-tsc");
    let project = dir.path().join("my-mod");
    let output = init(&project, &[], &[]);
    assert!(output.status.success(), "{}", stderr(&output));
    fs::write(
        project.join("src/main.ts"),
        r#"import { ctx, console, setTimeout } from "manivelle:env";

export function run(): number {
    const result = math.add({ left: 20, right: 22 });
    setTimeout(() => console.log(structuredClone(result.value)), 10);
    ctx.hot.save(() => ({ at: 1 }));
    return result.value;
}
"#,
    )
    .expect("write script");

    let Some(output) = support::run_tsc_project(&project) else {
        return;
    };

    assert!(
        output.status.success(),
        "tsc rejected the generated project\n{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn running_init_again_updates_the_declarations_and_keeps_the_authors_files() {
    let dir = TestCacheDir::new("cli-init-again");
    let project = dir.path().join("my-mod");
    assert!(init(&project, &[], &[]).status.success());
    fs::write(project.join("manivelle/manivelle.d.ts"), "stale").expect("stale types");
    fs::write(project.join("src/main.ts"), "// mine\n").expect("edit script");
    fs::write(project.join("tsconfig.json"), "{ \"mine\": true }").expect("edit tsconfig");

    let output = init(&project, &[], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    let types = fs::read_to_string(project.join("manivelle/manivelle.d.ts")).expect("types");
    assert!(types.contains("math"), "the declarations were refreshed");
    assert_eq!(
        fs::read_to_string(project.join("src/main.ts")).expect("script"),
        "// mine\n"
    );
    assert_eq!(
        fs::read_to_string(project.join("tsconfig.json")).expect("tsconfig"),
        "{ \"mine\": true }"
    );
}

#[test]
fn init_can_name_the_generated_files() {
    let dir = TestCacheDir::new("cli-init-names");
    let project = dir.path().join("my-mod");

    let output = init(
        &project,
        &["--types=game.d.ts", "--sdk", "game.sdk.ts"],
        &[],
    );

    assert!(output.status.success(), "{}", stderr(&output));
    assert!(project.join("manivelle/game.d.ts").is_file());
    assert!(project.join("manivelle/game.sdk.ts").is_file());
    let tsconfig = fs::read_to_string(project.join("tsconfig.json")).expect("tsconfig");
    assert!(tsconfig.contains("manivelle/game.d.ts"), "{tsconfig}");
}

#[test]
fn a_program_that_did_not_opt_in_is_reported_and_leaves_no_files() {
    let dir = TestCacheDir::new("cli-init-no-optin");
    let project = dir.path().join("my-mod");

    // The program exits cleanly without calling `Engine::setup_protocol`.
    let output = init(&project, &[], &["no-opt-in"]);

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("Engine::setup_protocol"),
        "{}",
        stderr(&output)
    );
    assert!(!project.exists());
}

#[test]
fn a_program_that_never_exits_is_stopped_at_the_timeout() {
    let dir = TestCacheDir::new("cli-init-timeout");
    let project = dir.path().join("my-mod");

    let output = Command::new(env!("CARGO_BIN_EXE_manivelle"))
        .env("MANIVELLE_TEST_HANG", "1")
        .arg("init")
        .arg(&project)
        .arg("--from")
        .arg(example_program("init_program"))
        .args(["--timeout", "1"])
        .output()
        .expect("run manivelle");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("did not export its SDK within 1 s"),
        "{}",
        stderr(&output)
    );
    assert!(!project.exists());
}

#[test]
fn file_names_that_could_leave_the_project_are_refused() {
    let dir = TestCacheDir::new("cli-init-bad-names");
    let project = dir.path().join("my-mod");

    for bad in [
        "../escaped.d.ts",
        "/tmp/escaped.d.ts",
        "sub/dir.d.ts",
        "plain.txt",
    ] {
        let output = init(&project, &["--types", bad], &[]);

        assert!(!output.status.success(), "{bad} was accepted");
        assert!(!project.exists(), "{bad} left files behind");
    }
    let same = init(
        &project,
        &["--types", "same.d.ts", "--sdk", "same.d.ts"],
        &[],
    );
    assert!(!same.status.success());
}

#[test]
fn the_program_is_a_path_and_is_not_looked_up_in_path() {
    let dir = TestCacheDir::new("cli-init-no-path-lookup");
    let project = dir.path().join("my-mod");

    let output = Command::new(env!("CARGO_BIN_EXE_manivelle"))
        .arg("init")
        .arg(&project)
        .args(["--from", "ls"])
        .current_dir(dir.path())
        .output()
        .expect("run manivelle");

    assert!(!output.status.success());
    assert!(
        stderr(&output).contains("pass the path to the program"),
        "{}",
        stderr(&output)
    );
}

#[cfg(unix)]
#[test]
fn a_symbolic_link_in_the_project_is_replaced_and_never_followed() {
    use std::os::unix::fs::symlink;

    let dir = TestCacheDir::new("cli-init-symlinks");
    let project = dir.path().join("my-mod");
    let victim = dir.path().join("victim.txt");
    let created = dir.path().join("must-not-exist.txt");
    fs::write(&victim, "PRECIOUS").expect("victim");
    fs::create_dir_all(project.join("manivelle")).expect("project");
    symlink(&victim, project.join("manivelle/manivelle.d.ts")).expect("link to a file");
    symlink(&created, project.join("tsconfig.json")).expect("dangling link");

    let output = init(&project, &[], &[]);

    assert!(output.status.success(), "{}", stderr(&output));
    assert_eq!(fs::read_to_string(&victim).expect("victim"), "PRECIOUS");
    assert!(
        !created.exists(),
        "a dangling link must not be written through"
    );
    let declarations = project.join("manivelle/manivelle.d.ts");
    assert!(
        !fs::symlink_metadata(&declarations)
            .expect("declarations")
            .file_type()
            .is_symlink(),
        "the link itself is replaced by a file"
    );
    assert!(
        fs::read_to_string(&declarations)
            .expect("declarations")
            .contains("math")
    );
}

#[test]
fn help_describes_both_commands() {
    let output = Command::new(env!("CARGO_BIN_EXE_manivelle"))
        .arg("--help")
        .output()
        .expect("run manivelle");

    let help = String::from_utf8_lossy(&output.stdout);
    assert!(output.status.success());
    assert!(help.contains("init") && help.contains("sdk"), "{help}");
}
