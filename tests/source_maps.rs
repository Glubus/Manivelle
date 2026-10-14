mod support;

use std::fs;
use std::path::Path;

use manivelle::{Engine, VmError, VmOptions};
use serde_json::json;

use support::TestCacheDir;

/// Type-only declarations and an enum shift every JavaScript line away from its
/// TypeScript line.
const SHAPES: &str = r#"type Point = { x: number; y: number };

interface Shape {
  area(): number;
}

enum Mode {
  Calm,
  Angry,
}

export function explode(point: Point, mode: Mode = Mode.Angry): number {
  const total: number = point.x + point.y;
  if (mode === Mode.Angry) {
    throw new RangeError("angry total");
  }
  return total;
}

export const run = (): number => explode({ x: 1, y: 2 });
"#;

fn engine() -> Engine {
    Engine::new(&VmOptions::default()).expect("create engine")
}

/// `line:column` of the first `needle` in `source`, 1-based, in UTF-16 units.
fn position_of(source: &str, needle: &str) -> String {
    let offset = source.find(needle).expect("needle in source");
    let before = &source[..offset];
    let line_start = before.rfind('\n').map_or(0, |newline| newline + 1);
    let line = before.matches('\n').count() + 1;
    let column = before[line_start..].encode_utf16().count() + 1;
    format!("{line}:{column}")
}

fn execution_details<T: std::fmt::Debug>(result: Result<T, VmError>) -> String {
    match result {
        Err(VmError::Execution { details }) => details,
        other => panic!("expected an execution error, got {other:?}"),
    }
}

fn transpile_details<T: std::fmt::Debug>(result: Result<T, VmError>) -> String {
    match result {
        Err(VmError::Transpile { details }) => details,
        other => panic!("expected a transpile error, got {other:?}"),
    }
}

fn assert_mapped(details: &str, locations: &[String]) {
    for location in locations {
        assert!(
            details.contains(location),
            "missing {location} in:\n{details}"
        );
    }
    assert!(
        !details.contains("manivelle://graph"),
        "unmapped frame in:\n{details}"
    );
}

#[test]
fn inline_stack_frames_point_at_typescript_lines() {
    let mut engine = engine();
    engine.load_script("shapes", SHAPES).expect("load script");

    let details = execution_details(engine.call::<f64>("shapes", "run", ()));

    assert_mapped(
        &details,
        &[
            format!(
                "at explode (shapes.ts:{})",
                position_of(SHAPES, "RangeError(")
            ),
            format!("(shapes.ts:{})", position_of(SHAPES, "explode({ x")),
        ],
    );
}

#[test]
fn columns_count_utf16_units_after_non_ascii_text() {
    let source = "export function greet(user?: { name: string }): string {\n  return \"héllo 😀 \" + user!.name;\n}\n";
    let mut engine = engine();
    engine.load_script("greet", source).expect("load script");

    let details = execution_details(engine.call::<String>("greet", "greet", ()));

    assert_mapped(
        &details,
        &[format!("greet.ts:{}", position_of(source, "user!.name"))],
    );
}

#[test]
fn project_frames_name_each_module_relative_to_the_root() {
    let root = TestCacheDir::new("source-maps-project");
    let project = root.path().join("project");
    let math = "export interface Pair {\n  left: number;\n  right: number;\n}\n\nexport function divide(pair: Pair): number {\n  if (pair.right === 0) {\n    throw new Error(\"division by zero\");\n  }\n  return pair.left / pair.right;\n}\n";
    let main = "import { divide, type Pair } from \"./lib/math\";\n\ntype Input = Pair;\n\nexport function run(): number {\n  const input: Input = { left: 1, right: 0 };\n  return divide(input);\n}\n";
    write_file(&project.join("lib/math.ts"), math);
    write_file(&project.join("main.ts"), main);
    let mut engine = engine();
    engine
        .load_project("project", project.join("main.ts"))
        .expect("load project");

    let details = execution_details(engine.call::<f64>("project", "run", ()));

    assert_mapped(
        &details,
        &[
            format!("at divide (lib/math.ts:{})", position_of(math, "Error(")),
            // QuickJS places a call to an imported binding at its last argument.
            format!("at run (main.ts:{})", position_of(main, "input);")),
        ],
    );
}

#[test]
fn top_level_error_during_load_is_mapped() {
    let source = "type Limit = number;\n\nconst limit: Limit = 3;\nif (limit > 2) {\n  throw new Error(\"limit too high\");\n}\nexport {};\n";

    let details = execution_details(engine().load_script("boot", source));

    assert_mapped(
        &details,
        &[format!("boot.ts:{}", position_of(source, "Error("))],
    );
}

#[test]
fn event_handler_error_is_mapped() {
    let source = "interface Tick {\n  n: number;\n}\n\nctx.on(\"tick\", (event: Tick) => {\n  throw new Error(\"tick \" + event.n);\n});\nexport {};\n";
    let mut engine = engine();
    engine.load_script("ticker", source).expect("load script");

    let details = execution_details(engine.emit("tick", &json!({ "n": 1 })));

    assert_mapped(
        &details,
        &[format!("ticker.ts:{}", position_of(source, "event.n"))],
    );
}

#[test]
fn async_export_rejection_is_mapped() {
    let source = "type Delay = number;\n\nexport async function later(delay: Delay): Promise<number> {\n  await Promise.resolve(delay);\n  throw new Error(\"too late\");\n}\n";
    let mut engine = engine();
    engine.load_script("async", source).expect("load script");

    let details = execution_details(engine.call::<f64>("async", "later", (1.0,)));

    assert_mapped(
        &details,
        &[format!("async.ts:{}", position_of(source, "Error("))],
    );
}

#[test]
fn frames_map_when_the_module_comes_from_the_disk_cache() {
    let cache = TestCacheDir::new("source-maps-cache");
    Engine::new(&cache.engine_options())
        .expect("create engine")
        .load_script("shapes", SHAPES)
        .expect("fill the cache");
    let mut restarted = Engine::new(&cache.engine_options()).expect("restart engine");
    restarted
        .load_script("shapes", SHAPES)
        .expect("load from the cache");

    let details = execution_details(restarted.call::<f64>("shapes", "run", ()));

    assert_mapped(
        &details,
        &[format!(
            "at explode (shapes.ts:{})",
            position_of(SHAPES, "RangeError(")
        )],
    );
}

#[test]
fn inline_syntax_error_names_file_line_and_column() {
    let source = "type Count = number;\nconst count: Count = ;\nexport {};\n";

    let details = transpile_details(engine().load_script("broken", source));

    assert!(
        details.starts_with(&format!("broken.ts:{}: ", position_of(source, ";\nexport"))),
        "{details}"
    );
}

#[test]
fn project_syntax_error_names_the_module_relative_to_the_root() {
    let root = TestCacheDir::new("source-maps-syntax");
    let project = root.path().join("project");
    let broken = "export function half(value: number): number {\n  return value / ;\n}\n";
    write_file(&project.join("lib/half.ts"), broken);
    write_file(
        &project.join("main.ts"),
        "import { half } from \"./lib/half\";\nexport const value = half(4);\n",
    );

    let details = transpile_details(engine().load_project("project", project.join("main.ts")));

    assert!(
        details.starts_with(&format!("lib/half.ts:{}: ", position_of(broken, ";\n}"))),
        "{details}"
    );
}

fn write_file(path: &Path, content: &str) {
    fs::create_dir_all(path.parent().expect("parent directory")).expect("create directory");
    fs::write(path, content).expect("write file");
}
