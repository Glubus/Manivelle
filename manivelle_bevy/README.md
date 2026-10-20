# manivelle_bevy

Run a [Manivelle](../README.md) `Engine` inside a Bevy app, and let the host functions of
its scripts read and change the Bevy `World`. Built for Bevy 0.20.

The crate adds no script function of its own: no `entity.spawn`, no `transform.set`. You
declare what your scripts may call, and each function reaches the world through the state
Manivelle lends it:

```rust
use bevy_ecs::prelude::{Resource, World};
use manivelle::{Caller, EngineError, host_fn_with_ctx};

#[derive(Resource, Default)]
struct Score(u32);

#[host_fn_with_ctx(name = "game.add_score")]
fn add_score(ctx: &Caller<'_>, amount: u32) -> Result<u32, EngineError> {
    ctx.with_state(|world: &mut World| {
        let mut score = world.resource_mut::<Score>();
        score.0 += amount;
        score.0
    })
}
```

```rust
use manivelle::{Engine, EngineOptions};
use manivelle_bevy::AppManivelleExt;

let mut engine = Engine::new(&EngineOptions::default())?;
AddScore::register(engine.registry())?;
engine.load_script("rules", "export function bonus() { return game.add_score(10); }")?;

app.init_resource::<Score>().add_manivelle(engine);
```

## What `add_manivelle` does

It stores the engine as a non-send resource (the engine cannot leave its thread) and adds
one exclusive system to `PreUpdate`. Each frame it:

- calls `poll_protocol`, so the `manivelle` tool can reach the game;
- advances the timers by the frame's `Time` delta (without a `Time` resource they stand
  still);
- calls `pump`, which delivers the answers of async host functions;
- calls `reload_changed` every `ManivelleSettings::hot_reload_every`, when you set it.

A failure is logged with `bevy_log`, and the frame goes on.

## Calling scripts from your own systems

`with_engine(world, |engine| ...)` takes the engine out of the world, lends the world to
the scripts for the duration of the closure, and puts the engine back, even if the closure
panics. Use it in an exclusive system:

```rust
fn bonus(world: &mut World) {
    let total = with_engine(world, |engine| engine.call::<u32>("rules", "bonus", ()));
}
```

It returns `None` when the world holds no engine, or when called inside another
`with_engine`.

## Limits

- A host function reaches the world only while the engine runs inside `with_engine` or the
  frame system. A script called any other way gets `EngineError::NoState`, which it can
  catch.
- Scripts run on the thread that runs exclusive systems, one at a time. This crate does not
  schedule them in parallel with other systems.
- Until a `manivelle` release that contains `Engine::with_state` is published, depend on
  this repository by path.
