//! A frame loop driving two scripts of one context group: timers on the host clock,
//! events, requests, an async host function answered from another thread, and a hot
//! reload that keeps the script's state.
//!
//! The scripts are real files in `examples/game_loop/`, type-checked by `tsc` against
//! `manivelle.d.ts` alone: it is generated from the contracts below (the `game` module,
//! the `frame` and `status` events) and checked in, and the program fails when it is
//! stale. After changing a contract, refresh it with
//! `cargo run --example game_loop --features derive -- --update-types`.
//!
//! Run: `cargo run --example game_loop --features derive`

use std::error::Error;
use std::fs;
use std::path::Path;
use std::thread;
use std::time::Duration;

use manivelle::{
    Engine, EngineOptions, HostContract, PendingCall, ScriptId, TsSchema, host_event, host_mod,
};

/// The per-frame event payload, encoded natively into a JavaScript object. `#[host_event]`
/// names the event, so `emit` takes `Frame::NAME` instead of a string that could be misspelled,
/// and the `frame` handler of `hud.ts` gets its parameter typed from this struct.
#[derive(TsSchema)]
#[host_event(name = "frame")]
struct Frame {
    dt: f64,
    tick: u32,
}

/// The `status` request: every script replies with a line of text.
#[derive(TsSchema)]
#[host_event(name = "status", reply = String)]
struct Status;

/// `import { game } from "game"`: the host functions of the scripts. A grouped script has
/// no host globals, so it reaches them through this import.
#[host_mod(namespace = "game", import = "game")]
mod game {
    use std::thread;
    use std::time::Duration;

    use manivelle::{EngineError, HostResolver, host_fn};

    /// The best score on record, answered by another thread.
    #[host_fn(name = "highScore")]
    pub fn high_score(_input: (), resolver: HostResolver<f64>) -> Result<(), EngineError> {
        thread::spawn(move || {
            thread::sleep(Duration::from_millis(5)); // pretend to read a save file
            resolver.resolve(1200.0).expect("resolve high score");
        });
        Ok(())
    }
}

/// The project: `spawner.ts`, `hud.ts`, their `tsconfig.json` and the generated declarations.
const PROJECT: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/examples/game_loop");

/// 50 frames per second of game time. A timer set from a timer callback counts from
/// the clock at that moment, so a frame that divides the period keeps waves on time.
const FRAME: Duration = Duration::from_millis(20);

fn main() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::new(&EngineOptions::default())?;
    engine
        .registry()
        .function::<game::HighScore>()?
        .callback::<Frame>()?
        .request::<Status>()?;

    // The scripts' types come from the contracts above: the checked-in declarations must
    // be what the registry generates now.
    let generated = engine.registry().dts()?;
    let path = Path::new(PROJECT).join("manivelle.d.ts");
    if std::env::args().any(|argument| argument == "--update-types") {
        fs::write(&path, &generated)?;
        println!("wrote {}", path.display());
        return Ok(());
    }
    // Git may check the file out with CRLF line endings.
    let checked_in = fs::read_to_string(&path)?.replace("\r\n", "\n");
    assert!(
        checked_in == generated,
        "{} is stale: run `cargo run --example game_loop --features derive -- --update-types`",
        path.display()
    );

    // Work on a copy of the project, so the reload below can edit a script.
    let scratch = tempfile::Builder::new()
        .prefix("manivelle-game-loop-")
        .tempdir()?;
    for entry in fs::read_dir(PROJECT)? {
        let entry = entry?;
        fs::copy(entry.path(), scratch.path().join(entry.file_name()))?;
    }
    let spawner = scratch.path().join("spawner.ts");
    engine.load_project_in("game", "spawner", &spawner)?;
    engine.load_project_in("game", "hud", scratch.path().join("hud.ts"))?;

    // 25 frames: the banner timer (500 ms) has fired, no wave yet.
    let mut tick = 0;
    run_frames(&engine, &mut tick, 25)?;
    let banner: String = engine.call("hud", "bannerText", ())?;
    let waves: f64 = engine.call("spawner", "wavesSpawned", ())?;
    println!(
        "after {tick} frames ({} ms): banner={banner:?} waves={waves}",
        tick * 20
    );
    assert_eq!((banner.as_str(), waves), ("playing", 0.0));

    // 75 more frames: two seconds of game time in all, so two waves.
    run_frames(&engine, &mut tick, 75)?;
    let waves: f64 = engine.call("spawner", "wavesSpawned", ())?;
    let frames: f64 = engine.call("hud", "frames", ())?;
    println!(
        "after {tick} frames ({} ms): waves={waves} hud frames={frames}",
        tick * 20
    );
    assert_eq!((waves, frames), (2.0, 100.0));

    // Edit the spawner on disk (version 1 -> 2) and reload what changed: `ctx.hot` hands
    // the wave count over. The edit changes the file's size, so the change is seen
    // whatever the clock resolution of the file system.
    let source = fs::read_to_string(&spawner)?;
    let edited = source.replace("const VERSION = 1;", "const VERSION = 2; // edited");
    assert_ne!(source, edited, "spawner.ts no longer declares version 1");
    fs::write(&spawner, edited)?;
    let report = engine.reload_changed();
    assert!(report.failed.is_empty(), "{:?}", report.failed);
    assert_eq!(report.reloaded, [ScriptId::from("spawner")]);
    let version: f64 = engine.call("spawner", "version", ())?;
    let waves: f64 = engine.call("spawner", "wavesSpawned", ())?;
    println!("reloaded spawner: version={version} waves={waves}");
    assert_eq!((version, waves), (2.0, 2.0));

    run_frames(&engine, &mut tick, 50)?; // one more second: the new version's timer
    let status: Vec<(ScriptId, String)> = engine.request(Status::NAME, &Status)?;
    for (script, reply) in &status {
        println!("status from {script}: {reply}");
    }
    assert_eq!(
        status,
        [
            // The reload kept the spawner's place in the delivery order.
            (ScriptId::from("spawner"), "spawner v2: waves=3".to_owned()),
            (
                ScriptId::from("hud"),
                "hud: Asteroids playing frames=150 tick=149".to_owned()
            ),
        ]
    );

    // The script awaits a host answer that arrives in a later frame.
    let best: PendingCall<f64> = engine.call_deferred("spawner", "bestScore", ())?;
    let mut waited = 0;
    while !best.is_finished() {
        thread::sleep(Duration::from_millis(1));
        engine.pump()?; // resume the script once the other thread has answered
        waited += 1;
        assert!(waited < 5_000, "the storage thread never answered");
    }
    let best = best.take().expect("finished")?;
    println!("best score: {best} (after {waited} pumps)");
    assert_eq!(best, 1500.0);

    Ok(())
}

/// One frame: move the script clock, then tell the scripts.
fn run_frames(engine: &Engine, tick: &mut u32, count: u32) -> Result<(), Box<dyn Error>> {
    for _ in 0..count {
        engine.advance_timers(FRAME)?;
        engine.emit(
            Frame::NAME,
            &Frame {
                dt: FRAME.as_secs_f64(),
                tick: *tick,
            },
        )?;
        *tick += 1;
    }
    Ok(())
}
