//! Rust <-> JS round-trip overhead on the same workloads:
//! mlua (Lua 5.4), QuickJS called directly, and the Manivelle `Engine`.
//! The gap between QuickJS direct and the engine is Manivelle's own overhead
//! (argument encoding, contract dispatch, execution budget).
//!
//! Run: `cargo run --release --example native_roundtrip`

#[path = "../benches/support/quickjs_json.rs"]
mod quickjs_json;

use std::hint::black_box;
use std::time::{Duration, Instant};

use manivelle::{Engine, EngineOptions};
use mlua::{Lua, LuaSerdeExt};
use rquickjs::{Context, Ctx, Function, Object, Runtime, Value as JsValue, prelude::Func};
use serde_json::{Value, json};

#[path = "../benches/support/workload.rs"]
mod workload;

use workload::{Inc, ScoreUpdate};

const OPS: u32 = 10_000;
const RUNS: usize = 5;
const SCRIPT_ID: &str = "demo";

const LUA_SOURCE: &str = r#"
function sum(a, b) return a + b end
function host_loop(n) local s = 0 for _ = 1, n do s = inc(s) end return s end
function step(x) return inc(x) end
function process(input)
  local total = 0
  for _, v in ipairs(input.stats) do total = total + v end
  return { id = input.id, total = total + input.position.x }
end
last = 0
function on_score(event) last = event.combo end
"#;

const JS_SOURCE: &str = r#"
function sum(a, b) { return a + b; }
function hostLoop(n) { let s = 0; for (let i = 0; i < n; i++) s = math.inc(s); return s; }
function step(x) { return math.inc(x); }
function process(input) {
  let total = 0;
  for (const v of input.stats) total += v;
  return { id: input.id, total: total + input.position.x };
}
let last = 0;
function onScore(event) { last = event.combo; }
"#;

const TS_SOURCE: &str = r#"
import { math, score } from "bench";

type Input = { id: number; stats: number[]; position: { x: number } };

export function sum(a: number, b: number): number { return a + b; }
export function hostLoop(n: number): number { let s = 0; for (let i = 0; i < n; i++) s = math.inc(s); return s; }
export function step(x: number): number { return math.inc(x); }
export function process(input: Input) {
  let total = 0;
  for (const v of input.stats) total += v;
  return { id: input.id, total: total + input.position.x };
}
let last = 0;
score.onUpdate((event: { combo: number }) => { last = event.combo; });
"#;

/// One way of running the demo workloads.
trait Backend {
    fn name(&self) -> &'static str;
    /// Rust -> JS: `sum(20, 22)`.
    fn sum(&self) -> f64;
    /// One Rust -> JS call whose script then calls the host `n` times.
    fn host_loop(&self, n: u32) -> f64;
    /// Rust -> JS -> Rust -> JS -> Rust: the script forwards `x` to the host.
    fn step(&self, x: f64) -> f64;
    /// Rust -> JS with an object argument and an object result.
    fn process(&self, input: &Value) -> Value;
    /// Rust -> JS event delivered to one handler.
    fn emit(&self, event: &Value);
}

struct LuaBackend {
    lua: Lua,
}

impl LuaBackend {
    fn new() -> Self {
        let lua = Lua::new();
        let inc = lua
            .create_function(|_, value: f64| Ok(value + 1.0))
            .expect("create lua inc");
        lua.globals().set("inc", inc).expect("set lua inc");
        lua.load(LUA_SOURCE).exec().expect("load lua source");
        Self { lua }
    }

    fn function(&self, name: &str) -> mlua::Function {
        self.lua.globals().get(name).expect("lua function")
    }
}

impl Backend for LuaBackend {
    fn name(&self) -> &'static str {
        "mlua"
    }

    fn sum(&self) -> f64 {
        self.function("sum").call((20.0, 22.0)).expect("lua sum")
    }

    fn host_loop(&self, n: u32) -> f64 {
        self.function("host_loop").call(n).expect("lua loop")
    }

    fn step(&self, x: f64) -> f64 {
        self.function("step").call(x).expect("lua step")
    }

    fn process(&self, input: &Value) -> Value {
        let arg = self.lua.to_value(input).expect("lua to_value");
        let result = self
            .function("process")
            .call::<mlua::Value>(arg)
            .expect("lua process");
        self.lua.from_value(result).expect("lua from_value")
    }

    fn emit(&self, event: &Value) {
        let arg = self.lua.to_value(event).expect("lua to_value");
        self.function("on_score")
            .call::<()>(arg)
            .expect("lua handler");
    }
}

struct QuickJsBackend {
    _runtime: Runtime,
    context: Context,
}

impl QuickJsBackend {
    fn new() -> Self {
        let runtime = Runtime::new().expect("create quickjs runtime");
        let context = Context::full(&runtime).expect("create quickjs context");
        context.with(|ctx| {
            // Same shape as the engine script's `math.inc`, for a fair comparison.
            let math = Object::new(ctx.clone()).expect("create quickjs math");
            math.set("inc", Func::from(|value: f64| value + 1.0))
                .expect("set quickjs inc");
            ctx.globals().set("math", math).expect("set quickjs math");
            ctx.eval::<(), _>(JS_SOURCE).expect("load quickjs source");
        });
        Self {
            _runtime: runtime,
            context,
        }
    }
}

fn js_function<'js>(ctx: &Ctx<'js>, name: &str) -> Function<'js> {
    ctx.globals().get(name).expect("quickjs function")
}

impl Backend for QuickJsBackend {
    fn name(&self) -> &'static str {
        "QuickJS direct"
    }

    fn sum(&self) -> f64 {
        self.context
            .with(|ctx| js_function(&ctx, "sum").call((20.0, 22.0)))
            .expect("quickjs sum")
    }

    fn host_loop(&self, n: u32) -> f64 {
        self.context
            .with(|ctx| js_function(&ctx, "hostLoop").call((n,)))
            .expect("quickjs loop")
    }

    fn step(&self, x: f64) -> f64 {
        self.context
            .with(|ctx| js_function(&ctx, "step").call((x,)))
            .expect("quickjs step")
    }

    fn process(&self, input: &Value) -> Value {
        self.context.with(|ctx| {
            let arg = quickjs_json::json_to_js(&ctx, input).expect("quickjs to js");
            let result: JsValue<'_> = js_function(&ctx, "process")
                .call((arg,))
                .expect("quickjs process");
            quickjs_json::js_to_json(&result).expect("quickjs to json")
        })
    }

    fn emit(&self, event: &Value) {
        self.context.with(|ctx| {
            let arg = quickjs_json::json_to_js(&ctx, event).expect("quickjs to js");
            js_function(&ctx, "onScore")
                .call::<_, ()>((arg,))
                .expect("quickjs handler");
        });
    }
}

struct EngineBackend {
    engine: Engine,
}

impl EngineBackend {
    fn new() -> Self {
        let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
        engine
            .registry()
            .function::<Inc>()
            .and_then(|registry| registry.callback::<ScoreUpdate>())
            .expect("register engine contracts");
        engine
            .load_script(SCRIPT_ID, TS_SOURCE)
            .expect("load engine script");
        Self { engine }
    }
}

impl Backend for EngineBackend {
    fn name(&self) -> &'static str {
        "Manivelle engine"
    }

    fn sum(&self) -> f64 {
        self.engine
            .call(SCRIPT_ID, "sum", (20.0, 22.0))
            .expect("engine sum")
    }

    fn host_loop(&self, n: u32) -> f64 {
        self.engine
            .call(SCRIPT_ID, "hostLoop", (n,))
            .expect("engine loop")
    }

    fn step(&self, x: f64) -> f64 {
        self.engine
            .call(SCRIPT_ID, "step", (x,))
            .expect("engine step")
    }

    fn process(&self, input: &Value) -> Value {
        self.engine
            .call(SCRIPT_ID, "process", (input,))
            .expect("engine process")
    }

    fn emit(&self, event: &Value) {
        self.engine
            .emit("score.update", event)
            .expect("engine emit");
    }
}

#[derive(Clone, Copy)]
enum Scenario {
    RustToJs,
    JsToRust,
    PingPong,
    ObjectRoundTrip,
    Event,
}

impl Scenario {
    const ALL: [Scenario; 5] = [
        Scenario::RustToJs,
        Scenario::JsToRust,
        Scenario::PingPong,
        Scenario::ObjectRoundTrip,
        Scenario::Event,
    ];

    fn label(self) -> &'static str {
        match self {
            Scenario::RustToJs => "Rust -> JS           sum(a, b)",
            Scenario::JsToRust => "JS -> Rust           inc(x) in a loop",
            Scenario::PingPong => "Rust -> JS -> Rust   step(x)",
            Scenario::ObjectRoundTrip => "Rust -> JS           object in/out",
            Scenario::Event => "Rust -> JS           event, 1 handler",
        }
    }
}

struct Workload {
    payload: Value,
    event: Value,
}

impl Workload {
    fn new() -> Self {
        Self {
            payload: json!({
                "id": 42,
                "name": "player-one",
                "tags": ["a", "b", "c", "d", "e"],
                "position": { "x": 1.5, "y": 2.5, "z": 3.5 },
                "stats": (1..=20).collect::<Vec<u32>>(),
            }),
            event: json!({ "combo": 7 }),
        }
    }

    /// Runs `OPS` operations of `scenario`.
    fn run(&self, backend: &dyn Backend, scenario: Scenario) {
        match scenario {
            Scenario::RustToJs => (0..OPS).for_each(|_| {
                black_box(backend.sum());
            }),
            Scenario::JsToRust => {
                black_box(backend.host_loop(OPS));
            }
            Scenario::PingPong => {
                black_box((0..OPS).fold(0.0, |x, _| backend.step(x)));
            }
            Scenario::ObjectRoundTrip => (0..OPS).for_each(|_| {
                black_box(backend.process(&self.payload));
            }),
            Scenario::Event => (0..OPS).for_each(|_| backend.emit(&self.event)),
        }
    }

    /// Median time of `RUNS` timed runs after one warm-up run.
    fn median(&self, backend: &dyn Backend, scenario: Scenario) -> Duration {
        self.run(backend, scenario);
        let mut samples = (0..RUNS)
            .map(|_| {
                let started = Instant::now();
                self.run(backend, scenario);
                started.elapsed()
            })
            .collect::<Vec<_>>();
        samples.sort();
        samples[RUNS / 2]
    }
}

/// Every backend must compute the same results before its timings mean anything.
fn verify(backend: &dyn Backend, workload: &Workload) {
    let name = backend.name();
    assert_eq!(backend.sum(), 42.0, "{name}: sum");
    assert_eq!(backend.host_loop(10), 10.0, "{name}: host loop");
    assert_eq!(backend.step(1.0), 2.0, "{name}: step");
    let processed = backend.process(&workload.payload);
    assert_eq!(processed["id"].as_f64(), Some(42.0), "{name}: process id");
    assert_eq!(
        processed["total"].as_f64(),
        Some(211.5),
        "{name}: process total"
    );
    backend.emit(&workload.event);
}

fn print_row(label: &str, cells: &[String]) {
    print!("{label:<40}");
    for cell in cells {
        print!("{cell:>16}");
    }
    println!();
}

fn format_ns_per_op(total: Duration) -> String {
    format!("{:.0} ns", total.as_nanos() as f64 / f64::from(OPS))
}

fn format_ms(total: Duration) -> String {
    format!("{:.2} ms", total.as_secs_f64() * 1_000.0)
}

fn main() {
    let backends: Vec<Box<dyn Backend>> = vec![
        Box::new(LuaBackend::new()),
        Box::new(QuickJsBackend::new()),
        Box::new(EngineBackend::new()),
    ];
    let workload = Workload::new();
    for backend in &backends {
        verify(backend.as_ref(), &workload);
    }
    let results = Scenario::ALL.map(|scenario| {
        backends
            .iter()
            .map(|backend| workload.median(backend.as_ref(), scenario))
            .collect::<Vec<_>>()
    });
    let names = backends
        .iter()
        .map(|backend| backend.name().to_owned())
        .collect::<Vec<_>>();

    println!("{OPS} operations per scenario, median of {RUNS} runs\n");
    println!("Cost per operation");
    print_row("", &names);
    for (scenario, row) in Scenario::ALL.iter().zip(&results) {
        print_row(
            scenario.label(),
            &row.iter().map(|d| format_ns_per_op(*d)).collect::<Vec<_>>(),
        );
    }
    println!("\nTotal for {OPS} operations");
    print_row("", &names);
    for (scenario, row) in Scenario::ALL.iter().zip(&results) {
        print_row(
            scenario.label(),
            &row.iter().map(|d| format_ms(*d)).collect::<Vec<_>>(),
        );
    }
}
