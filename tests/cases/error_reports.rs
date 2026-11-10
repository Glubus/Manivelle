//! How a failure of the host, or of a script, is shown to the other side.

use std::panic::{AssertUnwindSafe, catch_unwind};

use manivelle::{
    Engine, EngineError, EngineOptions, HostContract, HostContractKind, HostFunctionSignature,
};

struct Boom;

impl HostContract for Boom {
    const NAME: &'static str = "host.boom";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Boom {
    type Input = ();
    type Output = ();
}

/// A host panic reaches the script as an exception it can catch, and the same one
/// whichever way the panic came: from a host function or from the console's sink.
#[test]
fn a_host_panic_looks_the_same_to_the_script_wherever_it_happened() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function_with::<Boom>(|()| panic!("host function panic"))
        .expect("register");
    engine.set_console(|_, _, _| panic!("console sink panic"));
    engine
        .load_script(
            "script",
            r#"
            const seen: string[] = [];
            const describe = (error: unknown): string => {
                const { name, message } = error as Error;
                return `${name}|${message}`;
            };
            export function viaFunction(): void {
                try { host.boom(); } catch (error) { seen[0] = describe(error); }
            }
            export function viaConsole(): void {
                try { console.log("hello"); } catch (error) { seen[1] = describe(error); }
            }
            export function read(): string[] { return seen; }
            "#,
        )
        .expect("load");

    for export in ["viaFunction", "viaConsole"] {
        let outcome = catch_unwind(AssertUnwindSafe(|| engine.call::<()>("script", export, ())));
        assert!(outcome.is_err(), "{export}: the panic reaches the host");
    }
    let seen: Vec<String> = engine.call("script", "read", ()).expect("read");

    assert_eq!(seen.len(), 2, "{seen:?}");
    assert_eq!(seen[0], seen[1]);
    assert!(seen[0].contains("a host function panicked"), "{seen:?}");
}

#[test]
fn a_thrown_value_that_is_not_an_error_is_described_by_its_content() {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .load_script(
            "script",
            r#"
            export function throwsObject(): void { throw { code: 1, reason: "no" }; }
            export function throwsText(): void { throw "plain words"; }
            "#,
        )
        .expect("load");

    let object = engine.call::<()>("script", "throwsObject", ());
    let text = engine.call::<()>("script", "throwsText", ());

    assert!(
        matches!(&object, Err(EngineError::Execution { details })
            if details.contains(r#"{"code":1,"reason":"no"}"#)),
        "{object:?}"
    );
    assert!(
        matches!(&text, Err(EngineError::Execution { details }) if details.contains("plain words")),
        "{text:?}"
    );
}
