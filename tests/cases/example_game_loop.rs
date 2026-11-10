//! `examples/game_loop`: the program runs its asserts, including the one that fails when the
//! checked-in declarations differ from what the registry generates, and its scripts
//! type-check against those declarations.

use crate::support;

use std::path::Path;
use std::process::Command;

use support::{example_program, run_tsc_project, stderr, stdout};

/// The example's scripts, their `tsconfig.json` and the generated declarations.
fn project() -> &'static Path {
    Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/examples/game_loop"))
}

/// The program loads the scripts, reloads one after an edit, and compares the checked-in
/// `manivelle.d.ts` with the registry's. On a stale file it panics:
/// refresh the files with `cargo run --example game_loop --features derive -- --update-types`.
#[test]
fn the_program_runs_and_its_checked_in_declarations_are_current() {
    let output = Command::new(example_program("game_loop"))
        .output()
        .expect("run the example");
    assert!(
        output.status.success(),
        "stdout:\n{}\nstderr:\n{}",
        stdout(&output),
        stderr(&output)
    );
}

/// `hud.ts` and `spawner.ts` use the declarations generated from the Rust contracts alone (no
/// `paths`, no SDK file), `import { game } from "game"` included, so a
/// misspelled field or a wrong call is a type error.
#[test]
fn the_scripts_type_check_against_the_generated_declarations() {
    let Some(output) = run_tsc_project(project()) else {
        return;
    };
    assert!(
        output.status.success(),
        "tsc rejects examples/game_loop:\n{}{}",
        stdout(&output),
        stderr(&output)
    );
}
