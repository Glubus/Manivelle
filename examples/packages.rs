//! Loading the scripts other people write: packages, each with a `manivelle.kdl` manifest
//! that names its dependencies and the host functions it may call.
//!
//! `examples/packages/` holds two of them. `core` may heal; `greedy` depends on `core`
//! and lists no permission, so its call to `player.heal` is refused.
//!
//! Run: `cargo run --example packages --features derive`

use std::path::PathBuf;

use manivelle::{Engine, EngineError, EngineOptions, host_fn};

/// Heals the player; returns the health given.
#[host_fn(name = "player.heal")]
fn heal(amount: u32) -> u32 {
    amount
}

fn main() -> Result<(), EngineError> {
    let mut engine = Engine::new(&EngineOptions::default())?;
    engine.registry().function::<Heal>()?;

    // One directory of packages. They load dependencies first, whatever their order on
    // disk, and a package whose dependency is missing or failed is reported, not loaded.
    let packages = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("examples/packages");
    let load = engine.load_packages(&[packages])?;
    for package in &load.loaded {
        println!("loaded {} {}", package.name, package.version);
    }
    assert!(load.failed.is_empty(), "{:?}", load.failed);

    // A package is a script under its name.
    let healed: u32 = engine.call("core", "potion", ())?;
    assert_eq!(healed, 25);

    // `greedy` did not ask for `player.heal`: the call is refused where it is made, and
    // the error says which permission its manifest lacks.
    let refused = engine.call::<u32>("greedy", "steal_potion", ());
    let error = refused.expect_err("greedy has no permission to heal");
    println!("refused: {error}");
    assert!(error.to_string().contains("player.heal"), "{error}");
    Ok(())
}
