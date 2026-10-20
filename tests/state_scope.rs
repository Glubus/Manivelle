#![cfg(feature = "derive")]
//! `Engine::with_state` lends the host's `&mut` to the host functions that scripts reach.

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::time::Duration;

use manivelle::{Caller, Engine, EngineError, EngineOptions, host_fn_with_ctx};

#[derive(Default)]
struct World {
    score: u32,
}

#[host_fn_with_ctx(name = "world.add")]
fn add(ctx: &Caller<'_>, amount: u32) -> Result<u32, EngineError> {
    ctx.with_state(|world: &mut World| {
        world.score += amount;
        world.score
    })
}

fn engine_with_script(source: &str) -> Engine {
    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    Add::register(engine.registry()).unwrap();
    engine.load_script("rules", source).unwrap();
    engine
}

const RUN: &str = "export function run(n: number) { return world.add(n); }";

#[test]
fn a_host_function_mutates_the_lent_state_and_the_host_gets_it_back() {
    let mut engine = engine_with_script(RUN);
    let mut world = World::default();

    let (first, second) = engine
        .with_state(&mut world, |engine| {
            let first: u32 = engine.call("rules", "run", (5,)).unwrap();
            let second: u32 = engine.call("rules", "run", (7,)).unwrap();
            (first, second)
        })
        .unwrap();

    assert_eq!((first, second), (5, 12));
    assert_eq!(world.score, 12);
}

#[test]
fn events_and_timers_reach_the_state_too() {
    let mut engine = engine_with_script(
        r#"
        ctx.on("bonus", (n: number) => { world.add(n); });
        setTimeout(() => { world.add(100); }, 10);
        export function run(n: number) { return world.add(n); }
        "#,
    );
    let mut world = World::default();

    engine
        .with_state(&mut world, |engine| {
            engine.emit("bonus", &3_u32).unwrap();
            engine.advance_timers(Duration::from_millis(10)).unwrap();
        })
        .unwrap();

    assert_eq!(world.score, 103);
}

#[test]
fn a_call_outside_a_scope_throws_to_the_script() {
    let mut engine = engine_with_script(RUN);
    let error = engine.call::<u32>("rules", "run", (1,)).unwrap_err();
    assert!(
        error.to_string().contains("no host state is lent"),
        "{error}"
    );

    // The scope ends with its closure: the state is not reachable afterwards.
    let mut world = World::default();
    engine
        .with_state(&mut world, |engine| {
            engine.call::<u32>("rules", "run", (1,)).unwrap()
        })
        .unwrap();
    let error = engine.call::<u32>("rules", "run", (1,)).unwrap_err();
    assert!(
        error.to_string().contains("no host state is lent"),
        "{error}"
    );
    assert_eq!(world.score, 1);
}

#[test]
fn a_script_can_catch_the_missing_state() {
    let mut engine = engine_with_script(
        "export function run() { try { world.add(1); return 'lent'; } catch { return 'none'; } }",
    );
    let outside: String = engine.call("rules", "run", ()).unwrap();
    assert_eq!(outside, "none");

    let mut world = World::default();
    let inside = engine
        .with_state(&mut world, |engine| {
            engine.call::<String>("rules", "run", ()).unwrap()
        })
        .unwrap();
    assert_eq!(inside, "lent");
}

#[test]
fn a_state_of_another_type_names_the_type_that_was_asked() {
    let mut engine = engine_with_script(RUN);
    let mut other = String::from("not a world");

    let error = engine
        .with_state(&mut other, |engine| {
            engine.call::<u32>("rules", "run", (1,)).unwrap_err()
        })
        .unwrap();

    assert!(error.to_string().contains("World"), "{error}");
}

#[test]
fn scopes_do_not_nest_and_the_refusal_keeps_the_outer_state() {
    let mut engine = engine_with_script(RUN);
    let mut outer = World::default();
    let mut inner = World::default();

    let nested = engine
        .with_state(&mut outer, |engine| {
            let refused = engine.with_state(&mut inner, |_| ()).unwrap_err();
            let still_outer: u32 = engine.call("rules", "run", (2,)).unwrap();
            (refused, still_outer)
        })
        .unwrap();

    assert!(matches!(nested.0, EngineError::StateScopeActive));
    assert_eq!(nested.1, 2);
    assert_eq!((outer.score, inner.score), (2, 0));
}

#[test]
fn a_panic_in_the_scope_takes_the_state_back() {
    let mut engine = engine_with_script(RUN);
    let mut world = World::default();

    let panicked = catch_unwind(AssertUnwindSafe(|| {
        let _ = engine.with_state(&mut world, |_| panic!("host code panics in the scope"));
    }));
    assert!(panicked.is_err());

    let after = engine
        .with_state(&mut world, |engine| {
            engine.call::<u32>("rules", "run", (4,)).unwrap()
        })
        .unwrap();
    assert_eq!(after, 4);
}
