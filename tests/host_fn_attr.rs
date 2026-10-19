#![cfg(feature = "derive")]

use manivelle::{
    Engine, EngineError, EngineOptions, HostContract, HostFunction, TsSchema, host_fn, host_mod,
};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, TsSchema)]
struct FindUserInput {
    user_id: u64,
}

#[derive(Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct FindUserOutput {
    display_name: String,
}

#[host_fn(name = "user.find")]
fn find_user(input: FindUserInput) -> Result<FindUserOutput, EngineError> {
    Ok(FindUserOutput {
        display_name: format!("user {}", input.user_id),
    })
}

#[host_fn(name = "game.ticks")]
fn ticks() -> Result<f64, EngineError> {
    Ok(42.0)
}

#[test]
fn marker_carries_name_and_static_call() {
    assert_eq!(FindUser::NAME, "user.find");
    let output = FindUser::call(FindUserInput { user_id: 7 }).unwrap();
    assert_eq!(output.display_name, "user 7");
}

#[test]
fn scripts_call_generated_contracts() {
    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    engine
        .registry()
        .function::<FindUser>()
        .unwrap()
        .function::<Ticks>()
        .unwrap();
    engine
        .load_script(
            "s",
            r#"
            export function run() {
                return [user.find({ user_id: 7 }).displayName, game.ticks()];
            }
            "#,
        )
        .unwrap();
    let out: serde_json::Value = engine.call("s", "run", ()).unwrap();
    assert_eq!(out, serde_json::json!(["user 7", 42]));
}

#[host_mod(namespace = "math")]
mod math {
    use manivelle::{EngineError, host_fn};

    #[host_fn]
    pub fn add(input: (f64, f64)) -> Result<f64, EngineError> {
        Ok(input.0 + input.1)
    }

    #[host_fn(name = "sub")]
    pub fn subtract(input: (f64, f64)) -> Result<f64, EngineError> {
        Ok(input.0 - input.1)
    }

    #[host_fn(name = "debug.log", no_namespace)]
    pub fn log() -> Result<f64, EngineError> {
        Ok(1.0)
    }
}

#[host_mod]
mod physics {
    use manivelle::{EngineError, host_fn};

    #[host_fn]
    pub fn gravity() -> Result<f64, EngineError> {
        Ok(9.0)
    }
}

#[test]
fn host_mod_prefixes_names_unless_no_namespace() {
    assert_eq!(math::Add::NAME, "math.add");
    assert_eq!(math::Subtract::NAME, "math.sub");
    assert_eq!(math::Log::NAME, "debug.log");
    assert_eq!(physics::Gravity::NAME, "physics.gravity");
}
