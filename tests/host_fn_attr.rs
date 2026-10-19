#![cfg(feature = "derive")]

use manivelle::{
    Caller, Engine, EngineError, EngineOptions, HostContract, HostFunction, HostResolver,
    ScriptExport, ScriptId, TsSchema, host_event, host_fn, host_fn_async, host_fn_with_ctx,
    host_mod, script_export,
};
use serde::{Deserialize, Serialize};
use std::sync::Mutex;

#[derive(Deserialize, TsSchema)]
struct FindUserInput {
    user_id: u64,
}

#[derive(Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct FindUserOutput {
    display_name: String,
}

/// Finds a user by id.
///
/// Returns a display name.
#[host_fn(name = "user.find")]
fn find_user(input: FindUserInput) -> Result<FindUserOutput, EngineError> {
    Ok(FindUserOutput {
        display_name: format!("user {}", input.user_id),
    })
}

#[host_fn(name = "game.ticks")]
fn ticks() -> f64 {
    42.0
}

#[host_fn(name = "game.ping")]
fn ping() {}

#[test]
fn marker_carries_name_and_static_call() {
    assert_eq!(FindUser::NAME, "user.find");
    let output = FindUser::call(FindUserInput { user_id: 7 }).unwrap();
    assert_eq!(output.display_name, "user 7");
}

#[test]
fn doc_comments_reach_the_generated_typescript() {
    assert_eq!(
        FindUser::DOCS,
        "Finds a user by id.\n\nReturns a display name."
    );
    assert_eq!(Ticks::DOCS, "", "a function without comments has no docs");

    let engine = Engine::new(&EngineOptions::default()).unwrap();
    engine
        .registry()
        .function::<FindUser>()
        .unwrap()
        .function::<Ticks>()
        .unwrap();

    let declarations = engine.registry().dts().unwrap();
    assert!(
        declarations.contains(
            "  /**\n   * Finds a user by id.\n   *\n   * Returns a display name.\n   */\n  export function find("
        ),
        "{declarations}"
    );
    assert!(!declarations.contains("/** */"), "{declarations}");
    let sdk = engine.registry().sdk().unwrap();
    assert!(sdk.contains("* Finds a user by id."), "{sdk}");
    let descriptor = FindUser::descriptor();
    assert_eq!(descriptor.docs.as_deref(), Some(FindUser::DOCS));
}

#[test]
fn scripts_call_generated_contracts() {
    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    engine
        .registry()
        .function::<FindUser>()
        .unwrap()
        .function::<Ticks>()
        .unwrap()
        .function::<Ping>()
        .unwrap();
    engine
        .load_script(
            "s",
            r#"
            export function run() {
                game.ping();
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

#[host_fn_with_ctx(name = "host.whoami")]
fn whoami(ctx: &Caller<'_>) -> String {
    ctx.script_id().to_owned()
}

#[host_fn_with_ctx(name = "host.greet")]
fn greet(ctx: &Caller<'_>, input: String) -> Result<String, EngineError> {
    Ok(format!("{} greets {input}", ctx.script_id()))
}

#[test]
fn ctx_functions_see_which_script_called() {
    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    Whoami::register(engine.registry()).unwrap();
    Greet::register(engine.registry()).unwrap();
    for id in ["alpha", "beta"] {
        engine
            .load_script(
                id,
                "export function run() { return [host.whoami(), host.greet(\"x\")]; }",
            )
            .unwrap();
    }
    let alpha: Vec<String> = engine.call("alpha", "run", ()).unwrap();
    let beta: Vec<String> = engine.call("beta", "run", ()).unwrap();
    assert_eq!(alpha, ["alpha", "alpha greets x"]);
    assert_eq!(beta, ["beta", "beta greets x"]);
}

static PENDING: Mutex<Vec<(String, HostResolver<u32>)>> = Mutex::new(Vec::new());

#[host_fn_async(name = "scores.lookup")]
fn lookup_score(input: String, resolver: HostResolver<u32>) {
    PENDING.lock().unwrap().push((input, resolver));
}

#[host_fn_async(name = "scores.owner")]
fn score_owner(
    ctx: &Caller<'_>,
    _input: (),
    resolver: HostResolver<String>,
) -> Result<(), EngineError> {
    resolver.resolve(ctx.script_id().to_owned())
}

#[test]
fn async_functions_settle_through_their_resolver() {
    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    LookupScore::register(engine.registry()).unwrap();
    ScoreOwner::register(engine.registry()).unwrap();
    engine
        .load_script(
            "game",
            r#"
            export async function doubled(name: string): Promise<number> {
                return (await scores.lookup(name)) * 2;
            }
            export async function owner(): Promise<string> {
                return await scores.owner();
            }
            "#,
        )
        .unwrap();

    let pending = engine
        .call_deferred::<u32>("game", "doubled", ("ada",))
        .unwrap();
    let (name, resolver) = PENDING.lock().unwrap().pop().unwrap();
    resolver.resolve(21).unwrap();
    engine.pump().unwrap();
    assert_eq!(name, "ada");
    assert_eq!(pending.take().unwrap().unwrap(), 42);

    let owner = engine.call_deferred::<String>("game", "owner", ()).unwrap();
    engine.pump().unwrap();
    assert_eq!(owner.take().unwrap().unwrap(), "game");
}

/// A user was found.
///
/// Sent once per search hit.
#[host_event(name = "user.found")]
#[derive(Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct UserFound {
    user_id: u64,
}

#[derive(Serialize, TsSchema)]
#[host_event(name = "game.ask", reply = u32)]
struct Ask {
    n: u32,
}

#[host_mod(namespace = "world")]
mod world {
    use manivelle::{TsSchema, host_event};
    use serde::Serialize;

    #[derive(Serialize, TsSchema)]
    #[host_event(name = "spawned")]
    pub struct Spawned {
        pub id: u32,
    }
}

#[test]
fn events_are_declared_from_their_payload_type() {
    assert_eq!(UserFound::NAME, "user.found");
    assert_eq!(
        UserFound::DOCS,
        "A user was found.\n\nSent once per search hit."
    );
    assert_eq!(
        world::Spawned::NAME,
        "world.spawned",
        "host_mod prefixes events too"
    );

    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    engine
        .registry()
        .callback::<UserFound>()
        .unwrap()
        .request::<Ask>()
        .unwrap();
    engine
        .load_script(
            "s",
            r#"
            let seen = 0;
            ctx.on("user.found", (event: { userId: number }) => { seen = event.userId; });
            ctx.on("game.ask", (event: { n: number }) => event.n + 1);
            export const last = (): number => seen;
            "#,
        )
        .unwrap();

    assert_eq!(
        engine
            .emit(UserFound::NAME, &UserFound { user_id: 7 })
            .unwrap(),
        1
    );
    assert_eq!(engine.call::<f64>("s", "last", ()).unwrap(), 7.0);
    let replies: Vec<(ScriptId, u32)> = engine.request(Ask::NAME, &Ask { n: 41 }).unwrap();
    assert_eq!(replies, [(ScriptId::from("s"), 42)]);

    let declarations = engine.registry().dts().unwrap();
    assert!(
        declarations.contains(
            "  /**\n   * A user was found.\n   *\n   * Sent once per search hit.\n   */\n  \"user.found\": UserFound;"
        ),
        "{declarations}"
    );
    let sdk = engine.registry().sdk().unwrap();
    assert!(sdk.contains("* A user was found."), "{sdk}");
}

#[script_export(name = "double", args = (f64,), output = f64)]
struct Double;

#[script_export(name = "reset")]
struct Reset;

#[test]
fn script_exports_are_declared_with_an_attribute() {
    assert_eq!((Double::NAME, Reset::NAME), ("double", "reset"));

    let mut engine = Engine::new(&EngineOptions::default()).unwrap();
    engine
        .load_script(
            "s",
            "export const double = (n: number): number => n * 2;\nexport function reset(): void {}",
        )
        .unwrap();

    assert_eq!(engine.call_typed::<Double>("s", (21.0,)).unwrap(), 42.0);
    engine.call_typed::<Reset>("s", ()).unwrap();
}
