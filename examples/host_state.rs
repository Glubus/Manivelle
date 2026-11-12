//! Two things a game does with the state around its scripts:
//!
//! - lend its own `&mut World` to host functions with `Engine::with_state`, so a handler
//!   reaches it without a lock or a global;
//! - save what the scripts keep (`ctx.hot.save`) with `Engine::snapshot`, and hand it to
//!   the scripts of a later run with `Engine::restore`.
//!
//! Run: `cargo run --example host_state --features derive`

use manivelle::{Caller, Engine, EngineError, EngineOptions, StateSnapshot, host_fn};

/// What the game owns. Nothing here is `Send`, `Sync` or behind a lock.
#[derive(Default)]
struct World {
    gold: u32,
}

/// Gives the player gold and returns the new total. The `Caller` is how a host function
/// reaches the state lent for the duration of the call.
#[host_fn(name = "world.give_gold")]
fn give_gold(ctx: &Caller<'_>, amount: u32) -> Result<u32, EngineError> {
    ctx.with_state(|world: &mut World| {
        world.gold += amount;
        world.gold
    })
}

/// A quest that counts its own completions, and keeps the count across reloads and runs.
const QUEST: &str = r#"
let completed = (ctx.hot.data as { completed: number } | undefined)?.completed ?? 0;
ctx.hot.save(() => ({ completed }));

export function complete(): number {
  completed += 1;
  world.give_gold(10 * completed);
  return completed;
}
"#;

fn engine() -> Result<Engine, EngineError> {
    let engine = Engine::new(&EngineOptions::default())?;
    engine.registry().function::<GiveGold>()?;
    Ok(engine)
}

fn main() -> Result<(), EngineError> {
    // First run: the quest is completed twice.
    let mut world = World::default();
    let mut first = engine()?;
    first.load_script("quest", QUEST)?;
    first.with_state(&mut world, |engine| -> Result<(), EngineError> {
        engine.call::<u32>("quest", "complete", ())?;
        engine.call::<u32>("quest", "complete", ())?;
        Ok(())
    })??;
    assert_eq!(world.gold, 30);

    // The save file: plain JSON, from script id to what the script saved.
    let save = serde_json::to_string(&first.snapshot()?).expect("a snapshot is JSON");
    println!("saved: {save}");
    drop(first);

    // A later run: restore before loading, and the script starts from its saved state.
    let snapshot: StateSnapshot = serde_json::from_str(&save).expect("a snapshot is JSON");
    let mut second = engine()?;
    second.restore(snapshot)?;
    second.load_script("quest", QUEST)?;
    let completed = second.with_state(&mut world, |engine| {
        engine.call::<u32>("quest", "complete", ())
    })??;

    assert_eq!(completed, 3);
    assert_eq!(world.gold, 60);
    println!("completed {completed} times, {} gold", world.gold);
    Ok(())
}
