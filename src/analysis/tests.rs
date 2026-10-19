use std::fs;

use super::*;
use crate::contract::{HostContract, HostContractKind, HostFunctionSignature};
use crate::{PackageManifest, check_package};

struct Add;

impl HostContract for Add {
    const NAME: &'static str = "math.add";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Add {
    type Input = f64;
    type Output = f64;
}

struct Modify;

impl HostContract for Modify {
    const NAME: &'static str = "world.modify";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Modify {
    type Input = ();
    type Output = f64;
}

struct Ping;

impl HostContract for Ping {
    const NAME: &'static str = "game.ping";

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

fn contracts() -> ContractNames {
    ContractNames::from_descriptors(&[
        Add::descriptor(),
        Modify::descriptor(),
        Ping::descriptor(),
        export_descriptor("run"),
    ])
}

fn export_descriptor(name: &str) -> crate::HostContractDescriptor {
    let mut descriptor = Add::descriptor();
    descriptor.name = name.to_owned();
    descriptor.kind = HostContractKind::Export;
    descriptor
}

fn used(source: &str) -> ScriptUsage {
    analyze_source(source, "main.ts", &contracts())
}

fn names(usages: &[Usage]) -> Vec<&str> {
    usages.iter().map(|u| u.name.as_str()).collect()
}

#[test]
fn host_calls_and_listeners_are_found_with_their_line() {
    let usage = used(
        "export function run() {\n  ctx.on(\"game.ping\", () => {});\n  return math.add(1) + world.modify();\n}\n",
    );

    assert_eq!(names(&usage.functions), ["math.add", "world.modify"]);
    assert_eq!(names(&usage.events), ["game.ping"]);
    assert_eq!(usage.functions[0].line, 3);
    assert_eq!(usage.events[0].line, 2);
    assert!(usage.exports.contains("run"));
}

#[test]
fn a_local_with_the_name_of_a_namespace_is_not_the_host() {
    let usage = used(
        "const math = { add: (n: number) => n };\nexport const a = () => math.add(1);\nexport function b(world: { modify(): number }) { return world.modify(); }\n",
    );

    assert!(usage.functions.is_empty(), "{usage:?}");
    assert!(usage.unknown.is_empty(), "{usage:?}");
}

#[test]
fn a_misspelled_host_function_is_unknown_but_builtins_are_left_alone() {
    let usage = used("export const a = () => math.ad(1) + Math.max(1, 2) + JSON.stringify({});");

    assert_eq!(names(&usage.unknown), ["math.ad"]);
    assert!(usage.functions.is_empty());
}

#[test]
fn computed_access_into_a_namespace_is_reported_as_dynamic() {
    let usage =
        used("export const a = (name: string) => math[name](1);\nctx.on(someName, () => {});");

    assert_eq!(names(&usage.dynamic), ["math[...]", "ctx.on(...)"]);
}

#[test]
fn the_generated_sdk_and_the_env_module_are_read_like_globals() {
    let usage = used(
        "import { math, events, ctx } from \"./manivelle/manivelle.sdk\";\nimport { ctx as c } from \"manivelle:env\";\nexport function run() {\n  math.add(1);\n  events.game.ping(() => {});\n  c.on(\"game.ping\", () => {});\n  return ctx;\n}\n",
    );

    assert_eq!(names(&usage.functions), ["math.add"]);
    assert_eq!(names(&usage.events), ["game.ping", "game.ping"]);
}

#[test]
fn exports_are_collected_from_every_form() {
    let usage = used(
        "export function a() {}\nexport const b = 1, c = 2;\nfunction d() {}\nexport { d as e };\nexport default function () {}\nexport type T = number;\n",
    );

    let exports: Vec<&str> = usage.exports.iter().map(String::as_str).collect();
    assert_eq!(exports, ["a", "b", "c", "default", "e"]);
    assert!(!usage.re_exports_all);
    assert!(used("export * from \"./other\";").re_exports_all);
}

/// A package on disk with `permissions` and the files of `files`.
fn package(dir: &std::path::Path, permissions: &[&str], files: &[(&str, &str)]) -> PackageManifest {
    fs::create_dir_all(dir).unwrap();
    for (name, content) in files {
        let path = dir.join(name);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(path, content).unwrap();
    }
    let block = permissions.join("\n        ");
    let source = format!(
        "package \"p\" {{\n    version \"1.0.0\"\n    entry \"main.ts\"\n    permissions {{\n        {block}\n    }}\n}}\n"
    );
    PackageManifest::parse(&source, dir).unwrap()
}

fn descriptors() -> Vec<crate::HostContractDescriptor> {
    vec![
        Add::descriptor(),
        Modify::descriptor(),
        Ping::descriptor(),
        export_descriptor("run"),
    ]
}

fn messages(report: &PackageReport) -> Vec<String> {
    report.findings.iter().map(ToString::to_string).collect()
}

#[test]
fn a_package_that_uses_what_it_lists_is_clean() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = package(
        dir.path(),
        &["math.add", "game.ping"],
        &[
            (
                "main.ts",
                "import { helper } from \"./util\";\nctx.on(\"game.ping\", () => helper());\nexport function run() { return math.add(1); }\n",
            ),
            ("util.ts", "export const helper = () => 1;\n"),
        ],
    );

    let report = check_package(&manifest, &descriptors()).unwrap();

    assert!(report.findings.is_empty(), "{:?}", messages(&report));
}

#[test]
fn what_the_code_uses_and_the_permissions_do_not_list_is_an_error_across_files() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = package(
        dir.path(),
        &["math.add"],
        &[
            (
                "main.ts",
                "import \"./util\";\nexport const run = () => math.add(1);\n",
            ),
            ("util.ts", "export const grow = () => world.modify();\n"),
        ],
    );

    let report = check_package(&manifest, &descriptors()).unwrap();

    assert_eq!(report.errors(), 1, "{:?}", messages(&report));
    assert!(
        messages(&report)[0]
            .contains("util.ts:1: uses `world.modify`, which its permissions do not list"),
        "{:?}",
        messages(&report)
    );
    assert_eq!(report.missing_permissions, ["world.modify"]);
}

#[test]
fn typos_unused_permissions_and_missing_exports_are_reported() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = package(
        dir.path(),
        &["math.add", "world.modify", "game.pnig"],
        &[(
            "main.ts",
            "ctx.on(\"game.pnig\", () => {});\nexport const other = () => math.ad(1);\n",
        )],
    );

    let report = check_package(&manifest, &descriptors()).unwrap();
    let text = messages(&report).join("\n");

    assert!(
        text.contains(
            "permission `game.pnig` matches no contract of the program (did you mean `game.ping`?)"
        ),
        "{text}"
    );
    assert!(
        text.contains(
            "listens to `game.pnig`, which is no event of the program (did you mean `game.ping`?)"
        ),
        "{text}"
    );
    assert!(
        text.contains("`math.ad` is not a host function (did you mean `math.add`?)"),
        "{text}"
    );
    assert!(
        text.contains("warning: its permissions list `math.add`, which the code never uses"),
        "{text}"
    );
    assert!(
        text.contains("warning: its permissions list `world.modify`, which the code never uses"),
        "{text}"
    );
    assert!(
        text.contains("warning: the entry does not export `run`"),
        "{text}"
    );
}

#[test]
fn dynamic_access_silences_the_unused_permission_warnings_and_warns() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = package(
        dir.path(),
        &["math.add", "world.modify"],
        &[(
            "main.ts",
            "export function run(n: string) { return math[n](1); }\n",
        )],
    );

    let report = check_package(&manifest, &descriptors()).unwrap();
    let text = messages(&report).join("\n");

    assert!(
        text.contains("warning: main.ts:1: `math[...]` hides which host contract it reaches"),
        "{text}"
    );
    assert!(!text.contains("never uses"), "{text}");
    assert_eq!(report.errors(), 0, "{text}");
}

#[test]
fn a_missing_entry_and_an_unresolved_import_are_findings() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = package(dir.path(), &[], &[]);
    let report = check_package(&manifest, &descriptors()).unwrap();
    assert!(
        messages(&report)[0].contains("is not a file"),
        "{:?}",
        messages(&report)
    );

    let dir = tempfile::tempdir().unwrap();
    let manifest = package(dir.path(), &[], &[("main.ts", "import \"./nowhere\";\n")]);
    let report = check_package(&manifest, &descriptors()).unwrap();
    assert!(
        messages(&report)[0].contains("cannot read the project"),
        "{:?}",
        messages(&report)
    );
}

#[test]
fn the_generated_sdk_file_is_not_read_as_the_packages_code() {
    let dir = tempfile::tempdir().unwrap();
    let manifest = package(
        dir.path(),
        &["math.add"],
        &[
            (
                "main.ts",
                "import { math } from \"./manivelle/manivelle.sdk\";\nexport const run = () => math.add(1);\n",
            ),
            (
                "manivelle/manivelle.sdk.ts",
                "// manivelle: generated\nfunction __hostCall(name: string, input?: unknown) { return null; }\nexport const math = { add(n: number) { return __hostCall(\"math.add\", n); }, sub() { return __hostCall(\"world.modify\"); } };\n",
            ),
        ],
    );

    let report = check_package(&manifest, &descriptors()).unwrap();

    // `world.modify` appears in the SDK file only: it is not the package's usage.
    assert_eq!(report.errors(), 0, "{:?}", messages(&report));
    assert!(report.missing_permissions.is_empty());
}
