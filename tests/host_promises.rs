//! Async host functions: scripts await a Promise that Rust settles through a
//! `HostResolver`, delivered by `Engine::pump`.

mod support;

use std::fmt::Debug;
use std::fs;
use std::sync::{Arc, Mutex};
use std::thread;

use manivelle::{
    ContractValidation, Engine, EngineError, EngineOptions, HostContract, HostContractAbi,
    HostContractKind, HostFunctionSignature, HostResolver, InMemoryHostContractRegistry,
};
use serde_json::json;
use support::TestCacheDir;

/// `scores.lookup(name)`: a player's score, answered by the host later.
struct Score;

impl HostContract for Score {
    const NAME: &'static str = "scores.lookup";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Score {
    type Input = String;
    type Output = u32;
}

/// `host.whoami()`: the id of the calling script.
struct WhoAmI;

impl HostContract for WhoAmI {
    const NAME: &'static str = "host.whoami";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for WhoAmI {
    type Input = ();
    type Output = String;
}

type Calls<I, O> = Arc<Mutex<Vec<(I, HostResolver<O>)>>>;

const SCORE_SCRIPT: &str = r#"
const seen: number[] = [];
export async function doubled(name: string): Promise<number> {
  const score = await scores.lookup(name);
  seen.push(score);
  return score * 2;
}
export function observed(): number[] { return seen; }
"#;

fn validating() -> EngineOptions {
    EngineOptions {
        contract_validation: ContractValidation::InputsAndOutputs,
        ..EngineOptions::default()
    }
}

/// An engine whose `scores.lookup` keeps every call's input and resolver for the test
/// to settle.
fn score_engine(options: &EngineOptions) -> (Engine, Calls<String, u32>) {
    let engine = Engine::new(options).expect("create engine");
    let calls: Calls<String, u32> = Arc::default();
    let kept = Arc::clone(&calls);
    engine
        .registry()
        .async_function_with::<Score>(move |name, resolver| {
            kept.lock().expect("calls").push((name, resolver));
            Ok(())
        })
        .expect("register async function");
    (engine, calls)
}

/// The most recent call the host has not settled yet.
fn last_call<I, O>(calls: &Calls<I, O>) -> (I, HostResolver<O>) {
    calls.lock().expect("calls").pop().expect("a started call")
}

fn assert_failed_with<T: Debug>(result: Option<Result<T, EngineError>>, expected: &str) {
    match result {
        Some(Err(EngineError::Execution { details })) => {
            assert!(details.contains(expected), "{expected:?} not in {details}");
        }
        other => panic!("expected a failure mentioning {expected:?}, got {other:?}"),
    }
}

#[test]
fn a_resolution_reaches_the_script_only_when_the_engine_pumps() {
    let (mut engine, calls) = score_engine(&EngineOptions::default());
    engine
        .load_script("game", SCORE_SCRIPT)
        .expect("load script");

    let pending = engine
        .call_deferred::<u32>("game", "doubled", ("ada",))
        .expect("start call");
    let (name, resolver) = last_call(&calls);
    thread::spawn(move || resolver.resolve(21))
        .join()
        .expect("resolving thread")
        .expect("queue the reply");

    assert_eq!(name, "ada");
    assert!(!pending.is_finished());
    let before: Vec<u32> = engine.call("game", "observed", ()).expect("observe");
    assert!(
        before.is_empty(),
        "no script code ran before pump: {before:?}"
    );
    assert!(!pending.is_finished(), "only pump delivers host replies");

    engine.pump().expect("pump");

    assert_eq!(pending.take().expect("finished").expect("value"), 42);
    let after: Vec<u32> = engine.call("game", "observed", ()).expect("observe");
    assert_eq!(after, [21]);
}

#[test]
fn rejected_and_abandoned_calls_reject_the_script_promise() {
    let (mut engine, calls) = score_engine(&EngineOptions::default());
    engine
        .load_script("game", SCORE_SCRIPT)
        .expect("load script");

    let rejected = engine
        .call_deferred::<u32>("game", "doubled", ("eve",))
        .expect("start rejected call");
    last_call(&calls)
        .1
        .reject(EngineError::Execution {
            details: "eve is banned".to_owned(),
        })
        .expect("queue the rejection");
    let abandoned = engine
        .call_deferred::<u32>("game", "doubled", ("bob",))
        .expect("start abandoned call");
    drop(last_call(&calls));
    assert!(!rejected.is_finished() && !abandoned.is_finished());

    engine.pump().expect("pump");

    assert_failed_with(rejected.take(), "eve is banned");
    assert_failed_with(abandoned.take(), "dropped its resolver");
}

#[test]
fn failed_calls_reject_at_once_instead_of_throwing() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .async_function_with::<Score>(|name, _resolver| {
            Err(EngineError::Execution {
                details: format!("{name} is unknown"),
            })
        })
        .expect("register async function");
    engine
        .load_script(
            "game",
            r#"
            const message = async (score: () => Promise<number>) => {
              try { await score(); return "resolved"; }
              catch (error) { return (error as Error).message; }
            };
            export function failedHandler(): Promise<string> {
              return message(() => scores.lookup("zed"));
            }
            export function wrongInput(): Promise<string> {
              return message(() => scores.lookup(42 as any));
            }
            export function returnsPromise(): boolean {
              const pending = scores.lookup(42 as any);
              pending.catch(() => {});
              return pending instanceof Promise;
            }
            "#,
        )
        .expect("load script");

    let failed: String = engine.call("game", "failedHandler", ()).expect("call");
    let wrong: String = engine.call("game", "wrongInput", ()).expect("call");
    let returns_promise: bool = engine.call("game", "returnsPromise", ()).expect("call");

    assert!(failed.contains("zed is unknown"), "{failed}");
    assert_ne!(wrong, "resolved");
    assert!(
        returns_promise,
        "an invalid input rejects the returned Promise"
    );
}

#[test]
fn typed_async_functions_resolve_natively_and_when_validating() {
    for options in [EngineOptions::default(), validating()] {
        let (mut engine, calls) = score_engine(&options);
        engine
            .load_script("game", SCORE_SCRIPT)
            .expect("load script");

        let pending = engine
            .call_deferred::<u32>("game", "doubled", ("ada",))
            .expect("start call");
        last_call(&calls).1.resolve(5).expect("queue the reply");
        engine.pump().expect("pump");

        assert_eq!(pending.take().expect("finished").expect("value"), 10);
    }
}

/// A panic in an async handler panics out of the call that reached it, as for a
/// synchronous one: a script's `try`/`catch` cannot swallow it.
#[test]
fn a_panicking_async_handler_panics_out_of_the_call_and_leaves_the_engine_usable() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .async_function_with::<Score>(|_name, _resolver| panic!("async handler panicked"))
        .expect("register");
    engine
        .load_script(
            "game",
            r#"
            export function swallowed(): unknown {
                try { scores.lookup("ada"); return "returned"; } catch { return "caught"; }
            }
            export function fine(): number { return 1; }
            "#,
        )
        .expect("load script");

    let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.call::<String>("game", "swallowed", ())
    }));
    let fine: f64 = engine
        .call("game", "fine", ())
        .expect("engine stays usable");

    assert!(
        outcome.is_err(),
        "the panic must reach the host: {outcome:?}"
    );
    assert_eq!(fine, 1.0);
}

/// Starts `who()` in `alpha` and `beta`, settled synchronously by the handler, and
/// returns what each resolves to after one pump.
fn ids_resolved_by(engine: &mut Engine) -> [String; 2] {
    let script = "export async function who(): Promise<string> { return await host.whoami(); }";
    engine.load_script("alpha", script).expect("load alpha");
    engine.load_script("beta", script).expect("load beta");
    let alpha = engine
        .call_deferred::<String>("alpha", "who", ())
        .expect("start alpha");
    let beta = engine
        .call_deferred::<String>("beta", "who", ())
        .expect("start beta");
    assert!(
        !alpha.is_finished() && !beta.is_finished(),
        "a reply given during the call still waits for pump"
    );
    engine.pump().expect("pump");
    [alpha, beta].map(|pending| pending.take().expect("finished").expect("value"))
}

#[test]
fn caller_handlers_see_the_calling_script() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .async_function_with_caller::<WhoAmI>(|caller, (), resolver| {
            resolver.resolve(caller.script_id().to_owned())
        })
        .expect("register typed async function");

    assert_eq!(ids_resolved_by(&mut engine), ["alpha", "beta"]);
}

#[test]
fn replacing_a_script_cancels_its_calls_and_retired_functions_refuse_new_ones() {
    let (mut engine, calls) = score_engine(&EngineOptions::default());
    engine
        .load_script(
            "game",
            r#"
            ctx.hot.save(() => ({ lookup: scores.lookup }));
            export async function doubled(name: string): Promise<number> {
              return (await scores.lookup(name)) * 2;
            }
            "#,
        )
        .expect("load first version");
    let pending = engine
        .call_deferred::<u32>("game", "doubled", ("ada",))
        .expect("start call");
    let (_, resolver) = last_call(&calls);

    engine
        .load_script(
            "game",
            r#"
            const stale = (ctx.hot.data as { lookup: (name: string) => Promise<number> }).lookup;
            export async function viaStale(): Promise<string> {
              try { await stale("bob"); return "resolved"; }
              catch (error) { return (error as Error).message; }
            }
            export async function doubled(name: string): Promise<number> {
              return (await scores.lookup(name)) * 2;
            }
            "#,
        )
        .expect("load second version");

    assert!(resolver.is_cancelled());
    assert!(matches!(resolver.resolve(21), Err(EngineError::Cancelled)));
    assert!(matches!(pending.take(), Some(Err(EngineError::Cancelled))));
    let stale: String = engine.call("game", "viaStale", ()).expect("call");
    assert_eq!(stale, EngineError::Cancelled.to_string());
    assert!(
        calls.lock().expect("calls").is_empty(),
        "a retired version's function starts no call"
    );

    let fresh = engine
        .call_deferred::<u32>("game", "doubled", ("cy",))
        .expect("start call on the new version");
    last_call(&calls).1.resolve(4).expect("queue the reply");
    engine.pump().expect("pump");
    assert_eq!(fresh.take().expect("finished").expect("value"), 8);
}

#[test]
fn a_failed_reload_keeps_the_running_version_calls() {
    let (mut engine, calls) = score_engine(&EngineOptions::default());
    engine
        .load_script("game", SCORE_SCRIPT)
        .expect("load script");
    let pending = engine
        .call_deferred::<u32>("game", "doubled", ("ada",))
        .expect("start call");

    let broken = r#"export const broken: number = (() => { throw new Error("broken"); })();"#;
    assert!(engine.load_script("game", broken).is_err());

    let (_, resolver) = last_call(&calls);
    assert!(!resolver.is_cancelled());
    resolver.resolve(3).expect("queue the reply");
    engine.pump().expect("pump");
    assert_eq!(pending.take().expect("finished").expect("value"), 6);
}

#[test]
fn unloading_or_dropping_the_engine_cancels_waiting_resolvers() {
    let (mut engine, calls) = score_engine(&EngineOptions::default());
    engine.load_script("unloaded", SCORE_SCRIPT).expect("load");
    engine.load_script("kept", SCORE_SCRIPT).expect("load");
    let _unloaded_call = engine
        .call_deferred::<u32>("unloaded", "doubled", ("a",))
        .expect("start call");
    let (_, unloaded) = last_call(&calls);
    let _kept_call = engine
        .call_deferred::<u32>("kept", "doubled", ("b",))
        .expect("start call");
    let (_, kept) = last_call(&calls);

    engine.unload_script("unloaded").expect("unload");

    assert!(unloaded.is_cancelled());
    assert!(!kept.is_cancelled());

    drop(engine);

    assert!(kept.is_cancelled());
    assert!(matches!(kept.resolve(1), Err(EngineError::Cancelled)));
}

#[test]
fn settled_calls_release_their_promises() {
    let (mut engine, calls) = score_engine(&EngineOptions::default());
    engine
        .load_script("game", SCORE_SCRIPT)
        .expect("load script");
    let round = |engine: &Engine| {
        let pending: Vec<_> = (0..50)
            .map(|_| {
                engine
                    .call_deferred::<u32>("game", "doubled", ("ada",))
                    .expect("start call")
            })
            .collect();
        let started = std::mem::take(&mut *calls.lock().expect("calls"));
        for (index, (_, resolver)) in started.into_iter().enumerate() {
            if index % 2 == 0 {
                resolver.resolve(1).expect("queue the reply");
            }
        }
        engine.pump().expect("pump");
        assert!(pending.iter().all(|call| call.take().is_some()));
        engine.run_gc();
        engine.memory_stats().object_count
    };

    let baseline = round(&engine);
    for _ in 0..3 {
        round(&engine);
    }

    assert!(
        round(&engine) <= baseline,
        "settled calls keep no JavaScript object alive"
    );
}

#[test]
fn async_descriptors_declare_a_promise_and_sync_ones_serialize_unchanged() {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .async_function_with::<Score>(|_, _| Ok(()))
        .expect("register async function")
        .function_with::<WhoAmI>(|()| Ok(String::new()))
        .expect("register sync function");

    let score = registry
        .descriptor(Score::NAME)
        .expect("read")
        .expect("score");
    let who = registry
        .descriptor(WhoAmI::NAME)
        .expect("read")
        .expect("who");

    assert!(score.function.as_ref().expect("function").returns_promise);
    assert!(matches!(
        score.abi,
        HostContractAbi::Function {
            returns_promise: true,
            ..
        }
    ));
    let score_json = serde_json::to_value(&score).expect("serialize");
    assert_eq!(score_json["function"]["returns_promise"], json!(true));
    let who_json = serde_json::to_string(&who).expect("serialize");
    assert!(!who_json.contains("returns_promise"), "{who_json}");
}

fn score_registry() -> InMemoryHostContractRegistry {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .async_function_with::<Score>(|_, _| Ok(()))
        .expect("register async function");
    registry
}

#[test]
fn generated_declarations_type_async_functions_as_promises() {
    let dts = score_registry().dts().expect("render declarations");

    assert!(
        dts.contains("export function lookup(input: string): Promise<number>;"),
        "{dts}"
    );
    assert_tsc(
        "async-dts",
        &format!(
            "{dts}\nasync function total(): Promise<number> {{ const score: number = await scores.lookup(\"ada\"); return score + 1; }}\n"
        ),
        None,
    );
    // A Promise is not the number it resolves to (TS2322).
    assert_tsc(
        "async-dts-misuse",
        &format!("{dts}\nconst score: number = scores.lookup(\"ada\");\n"),
        Some("TS2322"),
    );
}

#[test]
fn generated_sdk_types_async_functions_as_promises() {
    let sdk = score_registry().sdk().expect("render SDK");

    assert!(
        sdk.contains("lookup(input: string): Promise<number>"),
        "{sdk}"
    );
    assert!(
        sdk.contains("\"scores.lookup\": { input: string; output: Promise<number>; };"),
        "{sdk}"
    );
    assert_tsc(
        "async-sdk",
        &format!(
            "{sdk}\nexport async function total(): Promise<number> {{ return (await scores.lookup(\"ada\")) + (await call(\"scores.lookup\", \"bob\")); }}\n"
        ),
        None,
    );
    assert_tsc(
        "async-sdk-misuse",
        &format!("{sdk}\nexport const score: number = scores.lookup(\"ada\");\n"),
        Some("TS2322"),
    );
}

/// Type-checks `source` when TypeScript is installed: it must pass, or fail with
/// `expected_error`.
fn assert_tsc(name: &str, source: &str, expected_error: Option<&str>) {
    let dir = TestCacheDir::new(name);
    let path = dir.path().join("usage.ts");
    fs::write(&path, source).expect("write usage");
    let Some(output) = support::run_tsc(&path) else {
        return;
    };
    let diagnostics = String::from_utf8_lossy(&output.stdout);
    match expected_error {
        None => assert!(output.status.success(), "tsc on {name}:\n{diagnostics}"),
        Some(code) => assert!(
            !output.status.success() && diagnostics.contains(code),
            "tsc on {name}:\n{diagnostics}"
        ),
    }
}
