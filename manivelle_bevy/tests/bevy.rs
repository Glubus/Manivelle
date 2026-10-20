//! A Bevy app drives a Manivelle engine, and host functions reach the app's `World`.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use bevy_app::App;
use bevy_ecs::prelude::{Resource, World};
use bevy_time::Time;
use manivelle::{Caller, Engine, EngineError, EngineOptions, host_fn_with_ctx};
use manivelle_bevy::{AppManivelleExt, ManivelleSettings, with_engine};

#[derive(Resource, Default)]
struct Score(u32);

#[host_fn_with_ctx(name = "game.add")]
fn add(ctx: &Caller<'_>, amount: u32) -> Result<u32, EngineError> {
    ctx.with_state(|world: &mut World| {
        let mut score = world.resource_mut::<Score>();
        score.0 += amount;
        score.0
    })
}

fn engine() -> Engine {
    let engine = Engine::new(&EngineOptions::default()).unwrap();
    Add::register(engine.registry()).unwrap();
    engine
}

fn app_with(engine: Engine) -> App {
    let mut app = App::new();
    app.init_resource::<Score>()
        .init_resource::<Time>()
        .add_manivelle(engine);
    app
}

fn score(app: &App) -> u32 {
    app.world().resource::<Score>().0
}

#[test]
fn a_host_function_changes_the_world_through_with_engine() {
    let mut engine = engine();
    engine
        .load_script(
            "rules",
            "export function run(n: number) { return game.add(n); }",
        )
        .unwrap();
    let mut app = app_with(engine);

    let total = with_engine(app.world_mut(), |engine| {
        engine.call::<u32>("rules", "run", (5,)).unwrap()
    });
    assert_eq!(total, Some(5));
    assert_eq!(score(&app), 5);

    // The engine went back into the world: a second call finds it.
    let total = with_engine(app.world_mut(), |engine| {
        engine.call::<u32>("rules", "run", (2,)).unwrap()
    });
    assert_eq!(total, Some(7));
}

#[test]
fn each_frame_advances_timers_by_the_time_delta() {
    let mut engine = engine();
    engine
        .load_script("rules", "setTimeout(() => { game.add(7); }, 10);")
        .unwrap();
    let mut app = app_with(engine);

    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_millis(9));
    app.update();
    assert_eq!(score(&app), 0, "9 ms is not yet 10 ms");

    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_millis(2));
    app.update();
    assert_eq!(score(&app), 7);
}

#[test]
fn timers_stand_still_without_a_time_resource() {
    let mut engine = engine();
    engine
        .load_script("rules", "setTimeout(() => { game.add(7); }, 0);")
        .unwrap();
    let mut app = App::new();
    app.init_resource::<Score>().add_manivelle(engine);

    app.update();

    assert_eq!(score(&app), 0);
}

#[test]
fn with_engine_is_none_without_an_engine() {
    let mut world = World::new();
    assert!(with_engine(&mut world, |_| ()).is_none());
}

#[test]
fn the_engine_comes_back_after_a_panic_in_with_engine() {
    let mut app = app_with(engine());

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        with_engine(app.world_mut(), |_| panic!("game code panics"))
    }));
    assert!(panicked.is_err());

    assert!(with_engine(app.world_mut(), |_| ()).is_some());
}

#[test]
#[should_panic(expected = "one app drives one engine")]
fn a_second_engine_is_refused() {
    let mut app = app_with(engine());
    app.add_manivelle(engine());
}

#[test]
fn hot_reload_runs_on_the_interval_and_not_before() {
    let dir = tempfile::tempdir().unwrap();
    let main = dir.path().join("main.ts");
    std::fs::write(&main, "export function value() { return 1; }").unwrap();
    let mut engine = engine();
    engine.load_project("rules", &main).unwrap();
    let mut app = app_with(engine);
    app.insert_resource(ManivelleSettings {
        hot_reload_every: Some(Duration::from_secs(1)),
    });
    // A longer file: the engine notices a change of size, whatever the clock resolution.
    std::fs::write(&main, "export function value() { return 22222; } // edited").unwrap();
    let value = |app: &mut App| {
        with_engine(app.world_mut(), |engine| {
            engine.call::<u32>("rules", "value", ()).unwrap()
        })
        .unwrap()
    };

    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_millis(500));
    app.update();
    assert_eq!(value(&mut app), 1, "half the interval: no check yet");

    app.world_mut()
        .resource_mut::<Time>()
        .advance_by(Duration::from_millis(600));
    app.update();
    assert_eq!(value(&mut app), 22222);
}
