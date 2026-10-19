//! A script can never import the modules of another script, in its group or outside it.

mod support;

use manivelle::Engine;
use support::group_engine;

/// A grouped script whose `me()` answers with the host's view of its caller.
const VICTIM: &str =
    "import { whoami } from 'host'; export function me(): string { return whoami(); }";

/// Every way a script could name the victim's modules, graph ids 0 to 3: its entry module,
/// its environment module and its instance of the `host` module.
fn victim_module_ids() -> Vec<String> {
    (0..4)
        .flat_map(|graph| {
            [
                format!("manivelle://graph/{graph}/victim"),
                format!("manivelle:env/{graph}"),
                format!("manivelle:host/{graph}/host"),
            ]
        })
        .collect()
}

fn assert_cannot_import_the_victim(
    load: impl Fn(&mut Engine, &str) -> Result<(), manivelle::EngineError>,
) {
    let mut engine = group_engine();
    engine
        .load_script_in("pack", "victim", VICTIM)
        .expect("load victim");

    for module in victim_module_ids() {
        let attacker = format!(
            "import * as v from '{module}'; export function steal(): unknown {{ return (v as any).me?.(); }}"
        );
        let loaded = load(&mut engine, &attacker);
        let stolen = loaded
            .as_ref()
            .ok()
            .and_then(|()| engine.call::<Option<String>>("attacker", "steal", ()).ok());
        assert!(
            loaded.is_err(),
            "importing `{module}` from another script loaded (steal() = {stolen:?})"
        );
    }
    assert_eq!(
        engine
            .call::<String>("victim", "me", ())
            .expect("victim still works"),
        "victim"
    );
}

#[test]
fn a_standalone_script_cannot_import_another_scripts_modules() {
    assert_cannot_import_the_victim(|engine, source| engine.load_script("attacker", source));
}

#[test]
fn a_script_cannot_import_the_modules_of_another_script_in_its_group() {
    assert_cannot_import_the_victim(|engine, source| {
        engine.load_script_in("pack", "attacker", source)
    });
}

/// `import()` is rejected when a script is compiled, but code a script builds at run
/// time can still call it.
#[test]
fn a_script_cannot_import_another_scripts_modules_at_run_time() {
    let mut engine = group_engine();
    engine
        .load_script_in("pack", "victim", VICTIM)
        .expect("load victim");
    engine
        .load_script(
            "attacker",
            r#"
            export async function steal(module: string): Promise<unknown> {
              const load = new Function("module", "return imp" + "ort(module)");
              const namespace = await load(module);
              return namespace.me?.();
            }
            "#,
        )
        .expect("load attacker");

    for module in victim_module_ids() {
        let stolen = engine.call::<Option<String>>("attacker", "steal", (module.as_str(),));
        assert!(
            stolen.is_err(),
            "imported `{module}` at run time: {stolen:?}"
        );
    }
}
