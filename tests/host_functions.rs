//! Host functions registered as closures: state, errors, caller identity, validation and
//! generated declarations.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use manivelle::{
    ContractValidation, Engine, EngineError, EngineOptions, HostContract, HostContractKind,
    HostFunction, HostFunctionSignature, InMemoryHostContractRegistry,
};

/// `counter.bump(name)`: counts calls, returns the running total.
struct Bump;

impl HostContract for Bump {
    const NAME: &'static str = "counter.bump";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Bump {
    type Input = String;
    type Output = u64;
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

/// `user.lookup(id)`: implemented statically as well, to compare registrations.
struct Lookup;

impl HostContract for Lookup {
    const NAME: &'static str = "user.lookup";
    const IMPORT_MODULE: &'static str = "host";
    const EXPORT_PATH: &'static [&'static str] = &["user", "lookup"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Lookup {
    type Input = u64;
    type Output = String;
}

impl HostFunction for Lookup {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(format!("user-{input}"))
    }
}

fn engine() -> Engine {
    Engine::new(&EngineOptions::default()).expect("create engine")
}

fn validating_engine() -> Engine {
    Engine::new(&EngineOptions {
        contract_validation: ContractValidation::InputsAndOutputs,
        ..EngineOptions::default()
    })
    .expect("create engine")
}

const WHO_SCRIPT: &str = r#"export function who(): string { return host.whoami(); }"#;

#[test]
fn closure_state_is_shared_by_every_script() {
    let mut engine = engine();
    let total = Arc::new(AtomicU64::new(0));
    let names = Arc::new(Mutex::new(Vec::new()));
    let (handler_total, handler_names) = (Arc::clone(&total), Arc::clone(&names));
    engine
        .registry()
        .function_with::<Bump>(move |name| {
            handler_names.lock().expect("names").push(name);
            Ok(handler_total.fetch_add(1, Ordering::SeqCst) + 1)
        })
        .expect("register closure");
    for id in ["alpha", "beta"] {
        let source =
            format!(r#"export function bump(): number {{ return counter.bump("{id}"); }}"#);
        engine.load_script(id, &source).expect("load script");
    }

    let totals: Vec<u64> = ["alpha", "beta", "alpha"]
        .into_iter()
        .map(|id| engine.call(id, "bump", ()).expect("call bump"))
        .collect();

    assert_eq!(totals, [1, 2, 3]);
    assert_eq!(total.load(Ordering::SeqCst), 3);
    assert_eq!(*names.lock().expect("names"), ["alpha", "beta", "alpha"]);
}

#[test]
fn closure_errors_are_catchable_in_scripts() {
    let mut engine = engine();
    engine
        .registry()
        .function_with::<Bump>(|name| {
            Err(EngineError::Execution {
                details: format!("{name} may not bump"),
            })
        })
        .expect("register closure");
    engine
        .load_script(
            "script",
            r#"
            export function caught(): string {
              try { counter.bump("mallory"); return "no error"; }
              catch (error) { return String((error as Error).message); }
            }
            export function uncaught(): number { return counter.bump("eve"); }
            "#,
        )
        .expect("load script");

    let caught: String = engine.call("script", "caught", ()).expect("call caught");
    let uncaught = engine.call::<f64>("script", "uncaught", ());

    assert!(caught.contains("mallory may not bump"), "{caught}");
    assert!(
        matches!(uncaught, Err(EngineError::Execution { ref details }) if details.contains("eve may not bump")),
        "{uncaught:?}"
    );
}

/// Loads `alpha` and `beta`, reloads `alpha`, and returns what `who()` answers for each.
fn ids_seen_by(engine: &mut Engine) -> [String; 3] {
    engine.load_script("alpha", WHO_SCRIPT).expect("load alpha");
    engine.load_script("beta", WHO_SCRIPT).expect("load beta");
    let alpha: String = engine.call("alpha", "who", ()).expect("call alpha");
    engine
        .load_script(
            "alpha",
            r#"export function who(): string { return "v2:" + host.whoami(); }"#,
        )
        .expect("reload alpha");
    let reloaded: String = engine
        .call("alpha", "who", ())
        .expect("call reloaded alpha");
    let beta: String = engine.call("beta", "who", ()).expect("call beta");
    [alpha, reloaded, beta]
}

#[test]
fn typed_caller_handlers_see_the_calling_script_across_reloads() {
    let mut engine = engine();
    engine
        .registry()
        .function_with_caller::<WhoAmI>(|caller, ()| Ok(caller.script_id().to_owned()))
        .expect("register closure");

    assert_eq!(ids_seen_by(&mut engine), ["alpha", "v2:alpha", "beta"]);
}

#[test]
fn caller_handlers_see_the_calling_script_when_validating() {
    let mut engine = validating_engine();
    engine
        .registry()
        .function_with_caller::<WhoAmI>(|caller, ()| Ok(caller.script_id().to_owned()))
        .expect("register closure");

    assert_eq!(ids_seen_by(&mut engine), ["alpha", "v2:alpha", "beta"]);
}

#[test]
fn stateful_typed_closures_run_when_validating() {
    let mut engine = validating_engine();
    let total = Arc::new(AtomicU64::new(0));
    let handler_total = Arc::clone(&total);
    engine
        .registry()
        .function_with::<Bump>(move |_| Ok(handler_total.fetch_add(1, Ordering::SeqCst) + 1))
        .expect("register closure");
    engine
        .load_script(
            "script",
            r#"
            export function twice(): number { counter.bump("a"); return counter.bump("b"); }
            export function invalid(): number { return counter.bump(42 as any); }
            "#,
        )
        .expect("load script");

    let second: u64 = engine.call("script", "twice", ()).expect("valid calls");
    let invalid = engine.call::<u64>("script", "invalid", ());

    assert_eq!(second, 2);
    assert!(
        matches!(invalid, Err(EngineError::Execution { .. })),
        "{invalid:?}"
    );
    assert_eq!(total.load(Ordering::SeqCst), 2);
}

/// A host handler that panics panics out of the call that reached it, as in any Rust
/// callback, and the engine is usable afterwards.
#[test]
fn a_panicking_host_function_panics_out_of_the_call_and_leaves_the_engine_usable() {
    let mut engine = engine();
    engine
        .registry()
        .function_with::<Bump>(|name| {
            assert!(!name.is_empty(), "host handler refused an empty name");
            Ok(1)
        })
        .expect("register closure");
    engine
        .load_script(
            "script",
            r#"
            export function boom(): number { return counter.bump(""); }
            export function swallowed(): number {
                try { return counter.bump(""); } catch { return -1; }
            }
            export function fine(): number { return counter.bump("a"); }
            "#,
        )
        .expect("load script");

    let boom = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.call::<f64>("script", "boom", ())
    }));
    let swallowed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        engine.call::<f64>("script", "swallowed", ())
    }));
    let fine: f64 = engine
        .call("script", "fine", ())
        .expect("engine stays usable");

    let message = |outcome: &std::thread::Result<_>| {
        outcome.as_ref().err().and_then(|payload| {
            payload.downcast_ref::<String>().cloned().or_else(|| {
                payload
                    .downcast_ref::<&str>()
                    .map(|text| (*text).to_owned())
            })
        })
    };
    assert!(
        message(&boom).is_some_and(|text| text.contains("refused an empty name")),
        "the call must panic with the handler's message"
    );
    assert!(
        swallowed.is_err(),
        "a script's try/catch must not swallow a Rust panic"
    );
    assert_eq!(fine, 1.0);
}

/// Declarations and SDK source of a registry holding only what `register` adds.
fn generated(
    register: impl FnOnce(
        &InMemoryHostContractRegistry,
    ) -> Result<&InMemoryHostContractRegistry, EngineError>,
) -> (String, String) {
    let registry = InMemoryHostContractRegistry::new();
    register(&registry).expect("register contract");
    (
        registry.dts().expect("render declarations"),
        registry.sdk().expect("render sdk"),
    )
}

#[test]
fn closures_generate_the_same_typed_contract_as_static_functions() {
    let from_static = generated(|registry| registry.function::<Lookup>());
    let from_closure =
        generated(|registry| registry.function_with::<Lookup>(|id| Ok(format!("closure-{id}"))));
    let from_caller = generated(|registry| {
        registry
            .function_with_caller::<Lookup>(|caller, id| Ok(format!("{}-{id}", caller.script_id())))
    });

    assert_eq!(from_closure, from_static);
    assert_eq!(from_caller, from_static);
}
