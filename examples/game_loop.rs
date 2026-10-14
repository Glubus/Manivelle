//! A frame loop driving two scripts of one context group: timers on the host clock,
//! events, requests, an async host function answered from another thread, and a hot
//! reload that keeps the script's state.
//!
//! Run: `cargo run --example game_loop --features derive`

use std::error::Error;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use manivelle::{
    Engine, HostContract, HostContractKind, HostFunctionSignature, HostResolver, PendingCall,
    TsSchema, VmOptions,
};

/// The per-frame event payload, encoded natively into a JavaScript object.
#[derive(TsSchema)]
struct Frame {
    dt: f64,
    tick: u32,
}

/// `import { highScore } from "game"`: an async host function, answered by the storage
/// thread.
struct HighScore;

impl HostContract for HighScore {
    const NAME: &'static str = "game.highScore";
    // A grouped script has no host globals: it reaches host functions through imports.
    const IMPORT_MODULE: &'static str = "game";
    const EXPORT_PATH: &'static [&'static str] = &["highScore"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for HighScore {
    type Input = ();
    type Output = f64;
}

/// Spawns a wave every second of game time, keeps its wave count across reloads, and
/// adds the stored high score to it.
fn spawner(version: u32) -> String {
    format!(
        r#"
        import {{ ctx, setTimeout }} from "manivelle:env";
        import {{ highScore }} from "game";

        let waves = (ctx.hot.data as {{ waves: number }} | undefined)?.waves ?? 0;
        ctx.hot.save(() => ({{ waves }}));

        function scheduleWave(): void {{
            setTimeout(() => {{ waves += 1; scheduleWave(); }}, 1000);
        }}
        scheduleWave();

        // A group shares one global object: the HUD reads this.
        (globalThis as any).gameTitle = "Asteroids";

        ctx.on("status", () => `spawner v{version}: waves=${{waves}}`);

        export function version(): number {{ return {version}; }}
        export function wavesSpawned(): number {{ return waves; }}
        export async function bestScore(): Promise<number> {{
            return (await highScore()) + waves * 100;
        }}
        "#
    )
}

/// Counts frames, shows a "get ready" banner for half a second, and answers the
/// `status` request.
const HUD: &str = r#"
import { ctx, setTimeout } from "manivelle:env";

let frameCount = 0;
let lastTick = -1;
let banner = "get ready";
setTimeout(() => { banner = "playing"; }, 500);

ctx.on("frame", (frame: { dt: number; tick: number }) => {
    frameCount += 1;
    lastTick = frame.tick;
});
ctx.on("status", () =>
    `hud: ${(globalThis as any).gameTitle} ${banner} frames=${frameCount} tick=${lastTick}`);

export function frames(): number { return frameCount; }
export function bannerText(): string { return banner; }
"#;

/// 50 frames per second of game time. A timer set from a timer callback counts from
/// the clock at that moment, so a frame that divides the period keeps waves on time.
const FRAME: Duration = Duration::from_millis(20);

fn main() -> Result<(), Box<dyn Error>> {
    let mut engine = Engine::new(&VmOptions::default())?;

    // The handler only hands the resolver over; the storage thread answers it.
    let (requests, storage_inbox) = mpsc::channel::<HostResolver<f64>>();
    let storage = thread::spawn(move || {
        for resolver in storage_inbox {
            thread::sleep(Duration::from_millis(5)); // pretend to read a save file
            resolver.resolve(1200.0).expect("resolve high score");
        }
    });
    engine
        .registry()
        .async_function_with::<HighScore>(move |(), resolver| {
            requests.send(resolver).expect("storage thread is running");
            Ok(())
        })?;

    engine.load_script_in("game", "spawner", &spawner(1))?;
    engine.load_script_in("game", "hud", HUD)?;

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

    // Reload the spawner with a new version: `ctx.hot` hands the wave count over.
    engine.load_script_in("game", "spawner", &spawner(2))?;
    let version: f64 = engine.call("spawner", "version", ())?;
    let waves: f64 = engine.call("spawner", "wavesSpawned", ())?;
    println!("reloaded spawner: version={version} waves={waves}");
    assert_eq!((version, waves), (2.0, 2.0));

    run_frames(&engine, &mut tick, 50)?; // one more second: the new version's timer
    let status: Vec<(&str, String)> = engine.request("status", &())?;
    for (script, reply) in &status {
        println!("status from {script}: {reply}");
    }
    assert_eq!(
        status,
        [
            // The reload kept the spawner's place in the delivery order.
            ("spawner", "spawner v2: waves=3".to_owned()),
            (
                "hud",
                "hud: Asteroids playing frames=150 tick=149".to_owned()
            ),
        ]
    );

    // The script awaits a host answer that arrives in a later frame.
    let best: PendingCall<f64> = engine.call_deferred("spawner", "bestScore", ())?;
    let mut waited = 0;
    while !best.is_finished() {
        thread::sleep(Duration::from_millis(1));
        engine.pump()?; // resume the script once the storage thread has answered
        waited += 1;
        assert!(waited < 5_000, "the storage thread never answered");
    }
    let best = best.take().expect("finished")?;
    println!("best score: {best} (after {waited} pumps)");
    assert_eq!(best, 1500.0);

    drop(engine); // drops the registry's sender: the storage thread ends
    storage.join().expect("storage thread");
    Ok(())
}

/// One frame: move the script clock, then tell the scripts.
fn run_frames(engine: &Engine, tick: &mut u32, count: u32) -> Result<(), Box<dyn Error>> {
    for _ in 0..count {
        engine.advance_timers(FRAME)?;
        engine.emit(
            "frame",
            &Frame {
                dt: FRAME.as_secs_f64(),
                tick: *tick,
            },
        )?;
        *tick += 1;
    }
    Ok(())
}
