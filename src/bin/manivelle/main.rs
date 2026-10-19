//! `manivelle`: the command line tool of Manivelle.
//!
//! - `manivelle init --from <program>` asks a program that opted in (it calls
//!   `Engine::setup_protocol`) for its scripting API, and sets up a script project
//!   around the TypeScript declarations it gets.
//! - `manivelle run` starts the target named by a package's `manivelle.kdl`, with its packages or
//!   without them (`--no-packages`), and stays connected to it.
//! - `manivelle sdk <descriptors.json> <dir>` writes the declarations and SDK from a
//!   descriptors file.

mod check;
mod export;
mod files;
mod init;
mod roots;
mod run;
mod sdk;

use std::error::Error;
use std::process::ExitCode;

use clap::{Parser, Subcommand};

type Failure = Box<dyn Error>;

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
    Init(init::InitArgs),
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
    Sdk(sdk::SdkArgs),
}

fn main() -> ExitCode {
    match dispatch(Cli::parse()) {
        Ok(()) => ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("error: {error}");
            ExitCode::FAILURE
        }
    }
}

fn dispatch(cli: Cli) -> Result<(), Failure> {
    match cli.command {
        Command::Init(args) => init::init(args),
        Command::Run(args) => run::run_target(args),
        Command::Check(args) => check::check(args),
        Command::Sdk(args) => sdk::sdk(args),
    }
}
