//! `import { x } from "module"` type-checks against the generated declarations alone: every
//! import module of the registry is a `declare module` block, no SDK file or `paths` needed.

use crate::support;

use std::fs;

use manivelle::{Engine, HostContract, HostContractKind, HostFunctionSignature, SdkFileNames};
use support::{Lookup, ScoreUpdate, TestCacheDir, WhoAmI, run_tsc_project, stdout};

/// `plain.go`: no `IMPORT_MODULE`, so no module declares it.
struct Plain;

impl HostContract for Plain {
    const NAME: &'static str = "plain.go";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Plain {
    type Input = ();
    type Output = f64;
}

/// `import { a } from "m"`: a function that is also the parent of `a.b`.
struct Mixed;

impl HostContract for Mixed {
    const NAME: &'static str = "m.a";
    const IMPORT_MODULE: &'static str = "m";
    const EXPORT_PATH: &'static [&'static str] = &["a"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Mixed {
    type Input = ();
    type Output = f64;
}

/// `import { a } from "m"; a.b()`.
struct MixedChild;

impl HostContract for MixedChild {
    const NAME: &'static str = "m.b";
    const IMPORT_MODULE: &'static str = "m";
    const EXPORT_PATH: &'static [&'static str] = &["a", "b"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for MixedChild {
    type Input = String;
    type Output = String;
}

fn engine() -> Engine {
    let engine = support::engine();
    engine
        .registry()
        .function_with_caller::<WhoAmI>(|caller, ()| Ok(caller.script_id().to_owned()))
        .and_then(|registry| {
            registry.async_function_with::<Lookup>(|(), resolver| resolver.resolve(1.0))
        })
        .and_then(|registry| registry.callback::<ScoreUpdate>())
        .and_then(|registry| registry.function_with::<Plain>(|()| Ok(1.0)))
        .and_then(|registry| registry.function_with::<Mixed>(|()| Ok(1.0)))
        .and_then(|registry| registry.function_with::<MixedChild>(Ok))
        .expect("register contracts");
    engine
}

/// Type-checks `main` as `main.ts` of a project that holds only the generated `manivelle.d.ts`:
/// no SDK file, no `paths`. `None` when TypeScript is not installed.
fn check(name: &str, main: &str) -> Option<String> {
    let dir = TestCacheDir::new(name);
    fs::write(
        dir.path().join("manivelle.d.ts"),
        engine().registry().dts().expect("dts"),
    )
    .expect("d.ts");
    fs::write(
        dir.path().join("tsconfig.json"),
        r#"{ "compilerOptions": { "target": "ES2022", "module": "ES2022", "moduleResolution": "Bundler",
              "lib": ["ES2023", "DOM"], "strict": true, "noEmit": true, "skipLibCheck": true },
              "include": ["main.ts", "manivelle.d.ts"] }"#,
    )
    .expect("tsconfig");
    fs::write(dir.path().join("main.ts"), main).expect("main.ts");
    let output = run_tsc_project(dir.path())?;
    let text = stdout(&output);
    Some(if output.status.success() {
        String::new()
    } else {
        text
    })
}

#[test]
fn an_import_module_is_declared_with_its_functions_namespaces_and_callbacks() {
    let dts = engine().registry().dts().expect("dts");
    let block = dts
        .split("\n\n")
        .find(|section| section.starts_with("declare module \"host\""))
        .unwrap_or_else(|| panic!("no `declare module \"host\"` in:\n{dts}"));
    assert_eq!(
        block,
        "declare module \"host\" {\n\
         \x20 export function lookup(input: void): Promise<number>;\n\
         \x20 export const score: {\n\
         \x20   onUpdate(handler: HostEventHandler<\"score.update\">): void;\n\
         \x20 };\n\
         \x20 export function whoami(input: void): string;\n\
         }"
    );
    let mixed = dts
        .split("\n\n")
        .find(|section| section.starts_with("declare module \"m\""))
        .expect("module m");
    assert_eq!(
        mixed,
        "declare module \"m\" {\n\
         \x20 export const a: {\n\
         \x20   b(input: string): string;\n\
         \x20   default(input: void): number;\n\
         \x20 };\n\
         }"
    );
    assert!(
        dts.contains("declare namespace plain {")
            && !block.contains("go(")
            && !mixed.contains("go("),
        "a contract without IMPORT_MODULE is a global only"
    );
}

#[test]
fn an_import_type_checks_against_the_declarations_alone() {
    let Some(errors) = check(
        "import-module-ok",
        r#"import { ctx } from "manivelle:env";
import { whoami, lookup, score } from "host";
import { a } from "m";

const me: string = whoami();
const best: Promise<number> = lookup();
score.onUpdate((points) => {
    const typed: number = points;
    void typed;
});
const viaChild: string = a.b("x");
const viaDefault: number = a.default();
void [ctx, me, best, viaChild, viaDefault];
"#,
    ) else {
        return;
    };
    assert_eq!(errors, "", "tsc rejects the imports");
}

#[test]
fn a_wrong_use_of_an_imported_function_is_a_type_error() {
    let Some(errors) = check(
        "import-module-wrong",
        r#"import { whoami, score } from "host";

const me: number = whoami();
score.onUpdate((points: string) => void points);
"#,
    ) else {
        return;
    };
    assert!(errors.contains("TS2322"), "{errors}");
    assert!(errors.contains("TS2345"), "{errors}");
}

#[test]
fn a_module_that_is_not_registered_does_not_resolve() {
    let Some(errors) = check(
        "import-module-unknown",
        "import { whoami } from \"hots\";\nvoid whoami;\n",
    ) else {
        return;
    };
    assert!(errors.contains("TS2307"), "{errors}");
}

#[test]
fn the_declarations_written_to_disk_hold_the_modules() {
    let dir = TestCacheDir::new("import-module-files");
    let written = engine()
        .registry()
        .write_sdk_files_with_names(
            dir.path(),
            &SdkFileNames {
                types: "api.d.ts".into(),
                sdk: "api.ts".into(),
            },
        )
        .expect("write");
    let dts = fs::read_to_string(written.types_path).expect("read");
    assert!(dts.contains("declare module \"host\" {"), "{dts}");
}
