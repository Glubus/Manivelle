//! `ScriptExports` in the generated declarations, checked by the real TypeScript compiler.

mod support;

use std::fs;
use std::path::Path;
use std::process::Output;

use manivelle::{Engine, EngineOptions, ScriptExport};
use support::TestCacheDir;

struct Double;

impl ScriptExport for Double {
    const NAME: &'static str = "double";
    type Args = (f64,);
    type Output = f64;
}

struct Reset;

impl ScriptExport for Reset {
    const NAME: &'static str = "reset";
    type Args = ();
    type Output = ();
}

/// A project holding the declarations of the registered exports and `main.ts`; `None`
/// when TypeScript is not installed (`npm ci`).
fn compile(dir: &Path, main: &str) -> Option<Output> {
    let engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .script_export::<Double>()
        .expect("register")
        .script_export::<Reset>()
        .expect("register");
    fs::create_dir_all(dir.join("src")).expect("src");
    fs::write(
        dir.join("manivelle.d.ts"),
        engine.registry().dts().expect("dts"),
    )
    .expect("d.ts");
    fs::write(
        dir.join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ES2022", "moduleResolution": "Bundler",
              "lib": ["ES2023", "DOM"], "strict": true, "noEmit": true, "skipLibCheck": true },
              "include": ["src", "manivelle.d.ts"] }"#,
    )
    .expect("tsconfig");
    fs::write(dir.join("src/main.ts"), main).expect("main.ts");
    support::run_tsc_project(dir)
}

fn output_text(output: &Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    )
}

#[test]
fn a_script_that_matches_its_exports_type_checks() {
    let dir = TestCacheDir::new("exports-tsc-ok");
    let Some(output) = compile(
        dir.path(),
        "export const double: ScriptExports[\"double\"] = (arg0) => arg0 * 2;\n\
         export async function reset(): Promise<void> {}\n\
         export const reset2: ScriptExports[\"reset\"] = async () => {};\n",
    ) else {
        return;
    };

    assert!(output.status.success(), "{}", output_text(&output));
}

#[test]
fn a_script_with_the_wrong_result_or_parameters_is_rejected() {
    let dir = TestCacheDir::new("exports-tsc-wrong");
    let Some(output) = compile(
        dir.path(),
        "export const double: ScriptExports[\"double\"] = (arg0) => \"twice\";\n",
    ) else {
        return;
    };
    assert!(
        !output.status.success(),
        "the wrong result type must not compile"
    );
    assert!(
        output_text(&output).contains("main.ts"),
        "{}",
        output_text(&output)
    );

    let dir = TestCacheDir::new("exports-tsc-params");
    let Some(output) = compile(
        dir.path(),
        "export const double: ScriptExports[\"double\"] = (a: string) => 1;\n",
    ) else {
        return;
    };
    assert!(
        !output.status.success(),
        "the wrong parameter type must not compile"
    );
}
