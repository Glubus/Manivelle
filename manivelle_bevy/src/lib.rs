//! Bevy integration for [Manivelle](https://docs.rs/manivelle): run an [`Engine`] inside a
//! Bevy app, and let the host functions of its scripts read and change the [`World`].
//!
//! The crate adds no script function of its own. A game declares the functions its scripts
//! may call, with Manivelle's `#[host_fn_with_ctx]`, and each one reaches the world through
//! `Caller::with_state::<World, _>`:
//!
//! ```ignore
//! #[host_fn_with_ctx(name = "game.add_score")]
//! fn add_score(ctx: &Caller<'_>, amount: u32) -> Result<u32, EngineError> {
//!     ctx.with_state(|world: &mut World| {
//!         let mut score = world.resource_mut::<Score>();
//!         score.0 += amount;
//!         score.0
//!     })
//! }
//!
//! app.add_manivelle(engine);
//! ```
//!
//! [`AppManivelleExt::add_manivelle`] stores the engine and adds the system that drives it
//! each frame. To call a script or emit an event from your own systems, use
//! [`with_engine`].

use std::time::Duration;

use bevy_app::{App, PreUpdate};
use bevy_ecs::prelude::{Local, Resource, World};
use bevy_log::error;
use bevy_time::Time;
use manivelle::{EngineError, ReloadReport};

pub use manivelle::Engine;

/// How [`AppManivelleExt::add_manivelle`] drives the engine.
#[derive(Resource, Debug, Clone, Default, PartialEq, Eq)]
pub struct ManivelleSettings {
    /// How often the engine checks the project files of its scripts and reloads those that
    /// changed. `None`, the default, never checks: a shipped game does not need it.
    pub hot_reload_every: Option<Duration>,
}

/// Adds Manivelle to a Bevy [`App`].
pub trait AppManivelleExt {
    /// Stores `engine` as a non-send resource and adds, in `PreUpdate`, the system that
    /// moves it forward each frame:
    ///
    /// - `poll_protocol`, so the `manivelle` tool can reach the game;
    /// - `advance_timers` by the frame's [`Time`] delta, when the app has the `Time`
    ///   resource (without it, timers stand still);
    /// - `pump`, which delivers the answers of async host functions;
    /// - `reload_changed`, when [`ManivelleSettings::hot_reload_every`] is set.
    ///
    /// The host functions of the scripts see the [`World`] during all of it. A failure is
    /// logged with `bevy_log` and the frame goes on.
    ///
    /// # Panics
    ///
    /// When the app already has an engine: one app drives one.
    fn add_manivelle(&mut self, engine: Engine) -> &mut Self;
}

impl AppManivelleExt for App {
    fn add_manivelle(&mut self, engine: Engine) -> &mut Self {
        assert!(
            !self.world().contains_non_send::<Engine>(),
            "`add_manivelle` was already called: one app drives one engine"
        );
        self.init_resource::<ManivelleSettings>();
        self.world_mut().insert_non_send(engine);
        self.add_systems(PreUpdate, drive_scripts);
        self
    }
}

/// Runs `f` with the app's [`Engine`] and lends it the `world`: every host function the
/// engine reaches inside `f` reads and changes that world through
/// `Caller::with_state::<World, _>`.
///
/// Returns `None` when `world` holds no engine: [`AppManivelleExt::add_manivelle`] was not
/// called, or this runs inside another `with_engine`, which already took it.
///
/// The engine leaves the world while `f` runs and comes back afterwards, even when `f`
/// panics.
pub fn with_engine<R>(world: &mut World, f: impl FnOnce(&mut Engine) -> R) -> Option<R> {
    let mut lent = Lent {
        engine: Some(world.remove_non_send::<Engine>()?),
        world,
    };
    let engine = lent.engine.as_mut()?;
    let result = engine.with_state(&mut *lent.world, f);
    // The engine was just taken out of the world, so no scope of it can be running.
    Some(result.expect("an engine out of the world has no state scope running"))
}

/// The engine taken out of the world, put back when this drops, on every way out.
struct Lent<'w> {
    engine: Option<Engine>,
    world: &'w mut World,
}

impl Drop for Lent<'_> {
    fn drop(&mut self) {
        if let Some(engine) = self.engine.take() {
            self.world.insert_non_send(engine);
        }
    }
}

/// The frame system: an exclusive one, because host functions need the whole world.
fn drive_scripts(world: &mut World, mut since_reload: Local<Duration>) {
    let delta = world.get_resource::<Time>().map(Time::delta);
    let reload_every = world
        .get_resource::<ManivelleSettings>()
        .and_then(|settings| settings.hot_reload_every);
    *since_reload += delta.unwrap_or_default();
    let reload = reload_every.is_some_and(|every| *since_reload >= every);
    if reload {
        *since_reload = Duration::ZERO;
    }

    with_engine(world, |engine| {
        engine.poll_protocol();
        if let Some(delta) = delta {
            log_failure(engine.advance_timers(delta));
        }
        log_failure(engine.pump());
        if reload {
            log_reload(engine.reload_changed());
        }
    });
}

fn log_failure<T>(result: Result<T, EngineError>) {
    if let Err(error) = result {
        error!("manivelle: {error}");
    }
}

fn log_reload(report: ReloadReport) {
    for (script, failure) in report.failed {
        error!("manivelle: reloading `{script}` failed: {failure}");
    }
    for (script, failure) in report.dispose_failed {
        error!("manivelle: the old version of `{script}` failed to clean up: {failure}");
    }
}
