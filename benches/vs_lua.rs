//! Same workloads on mlua (Lua 5.4), raw rquickjs and Manivelle.
//!
//! `quickjs_raw` is the floor Manivelle can reach on QuickJS: direct `Function::call`
//! with native value conversion. `manivelle_engine` is the single-thread [`Engine`].
//! The Manivelle gap to `quickjs_raw` is our own overhead; the raw QuickJS gap to mlua
//! is the interpreter difference.

use std::hint::black_box;
use std::time::Duration;

use criterion::{
    BenchmarkGroup, Criterion, criterion_group, criterion_main, measurement::WallTime,
};
use manivelle::{
    Engine, HostCallback, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
    VmError, VmOptions,
};
use mlua::{Lua, LuaSerdeExt};
use rquickjs::{Context, Ctx, Function, Object, Runtime, Value as JsValue, prelude::Func};
use serde_json::{Value, json};

#[path = "support/quickjs_json.rs"]
mod quickjs_json;

use quickjs_json::{js_to_json, json_to_js};

const HOST_CALLS: u32 = 1_000;
const FIB_N: u32 = 20;

const LUA_SOURCE: &str = r#"
function sum(a, b) return a + b end
function process(input)
  local total = 0
  for _, v in ipairs(input.stats) do total = total + v end
  return { id = input.id, total = total + input.position.x }
end
function loop(n)
  local s = 0
  for _ = 1, n do s = inc(s) end
  return s
end
function fib(n) if n < 2 then return n end return fib(n - 1) + fib(n - 2) end
last = 0
function on_score(event) last = event.combo end
"#;

const JS_SOURCE: &str = r#"
function sum(a, b) { return a + b; }
function process(input) {
  let total = 0;
  for (const v of input.stats) total += v;
  return { id: input.id, total: total + input.position.x };
}
function loop(n) { let s = 0; for (let i = 0; i < n; i++) s = math.inc(s); return s; }
function fib(n) { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
let last = 0;
function onScore(event) { last = event.combo; }
"#;

const TS_SOURCE: &str = r#"
import { math, score } from "bench";

type Input = { id: number; stats: number[]; position: { x: number } };

export function sum(a: number, b: number): number { return a + b; }
export function process(input: Input) {
  let total = 0;
  for (const v of input.stats) total += v;
  return { id: input.id, total: total + input.position.x };
}
export function loop(n: number): number { let s = 0; for (let i = 0; i < n; i++) s = math.inc(s); return s; }
export function fib(n: number): number { return n < 2 ? n : fib(n - 1) + fib(n - 2); }
let last = 0;
score.onUpdate((event: { combo: number }) => { last = event.combo; });
export function readLast(): number { return last; }
"#;

fn payload() -> Value {
    json!({
        "id": 42,
        "name": "player-one",
        "tags": ["a", "b", "c", "d", "e"],
        "position": { "x": 1.5, "y": 2.5, "z": 3.5 },
        "stats": (1..=20).collect::<Vec<u32>>(),
    })
}

struct Inc;

impl HostContract for Inc {
    const NAME: &'static str = "math.inc";
    const IMPORT_MODULE: &'static str = "bench";
    const EXPORT_PATH: &'static [&'static str] = &["math", "inc"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Inc {
    type Input = f64;
    type Output = f64;
}

impl HostFunction for Inc {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(input + 1.0)
    }
}

struct ScoreUpdate;

impl HostContract for ScoreUpdate {
    const NAME: &'static str = "score.update";
    const IMPORT_MODULE: &'static str = "bench";
    const EXPORT_PATH: &'static [&'static str] = &["score", "onUpdate"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for ScoreUpdate {
    type Payload = Value;
}

struct LuaBench {
    lua: Lua,
}

impl LuaBench {
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

struct QuickJsBench {
    runtime: Runtime,
    context: Context,
}

impl QuickJsBench {
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
        Self { runtime, context }
    }
}

fn js_function<'js>(ctx: &Ctx<'js>, name: &str) -> Function<'js> {
    ctx.globals().get(name).expect("quickjs function")
}

fn manivelle_engine() -> Engine {
    let mut engine = Engine::new(&VmOptions::default()).expect("create manivelle engine");
    engine
        .registry()
        .function::<Inc>()
        .and_then(|registry| registry.callback::<ScoreUpdate>())
        .expect("register bench contracts");
    engine
        .load_script("bench", TS_SOURCE)
        .expect("load manivelle engine source");
    engine
}

/// Every runtime compared by the benchmark groups.
struct Backends {
    lua: LuaBench,
    qjs: QuickJsBench,
    engine: Engine,
}

fn configure(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.sample_size(30);
    group.warm_up_time(Duration::from_secs(1));
    group.measurement_time(Duration::from_secs(3));
}

fn bench_call_scalar(c: &mut Criterion, backends: &Backends) {
    let Backends { lua, qjs, engine } = backends;
    let mut group = c.benchmark_group("call_scalar");
    configure(&mut group);
    group.bench_function("mlua", |b| {
        let sum = lua.function("sum");
        b.iter(|| black_box(sum.call::<f64>((20.0, 22.0)).expect("lua sum")));
    });
    group.bench_function("quickjs_raw", |b| {
        b.iter(|| {
            qjs.context.with(|ctx| {
                black_box(
                    js_function(&ctx, "sum")
                        .call::<_, f64>((20.0, 22.0))
                        .expect("quickjs sum"),
                )
            })
        });
    });
    group.bench_function("manivelle_engine", |b| {
        b.iter(|| {
            black_box(
                engine
                    .call::<f64>("bench", "sum", (20.0, 22.0))
                    .expect("engine sum"),
            )
        });
    });
    group.finish();
}

fn bench_call_object(c: &mut Criterion, backends: &Backends) {
    let Backends { lua, qjs, engine } = backends;
    let input = payload();
    let mut group = c.benchmark_group("call_object_roundtrip");
    configure(&mut group);
    group.bench_function("mlua", |b| {
        let process = lua.function("process");
        b.iter(|| {
            let arg = lua.lua.to_value(&input).expect("lua to_value");
            let result = process.call::<mlua::Value>(arg).expect("lua process");
            black_box(lua.lua.from_value::<Value>(result).expect("lua from_value"))
        });
    });
    group.bench_function("quickjs_raw", |b| {
        b.iter(|| {
            qjs.context.with(|ctx| {
                let arg = json_to_js(&ctx, &input).expect("quickjs to js");
                let result = js_function(&ctx, "process")
                    .call::<_, JsValue<'_>>((arg,))
                    .expect("quickjs process");
                black_box(js_to_json(&result).expect("quickjs to json"))
            })
        });
    });
    group.bench_function("manivelle_engine", |b| {
        b.iter(|| {
            black_box(
                engine
                    .call::<Value>("bench", "process", (&input,))
                    .expect("engine process"),
            )
        });
    });
    group.finish();
}

fn bench_host_calls(c: &mut Criterion, backends: &Backends) {
    let Backends { lua, qjs, engine } = backends;
    let mut group = c.benchmark_group("script_calls_host_1000x");
    configure(&mut group);
    group.bench_function("mlua", |b| {
        let run = lua.function("loop");
        b.iter(|| black_box(run.call::<f64>(HOST_CALLS).expect("lua loop")));
    });
    group.bench_function("quickjs_raw", |b| {
        b.iter(|| {
            qjs.context.with(|ctx| {
                black_box(
                    js_function(&ctx, "loop")
                        .call::<_, f64>((HOST_CALLS,))
                        .expect("quickjs loop"),
                )
            })
        });
    });
    group.bench_function("manivelle_engine", |b| {
        b.iter(|| {
            black_box(
                engine
                    .call::<f64>("bench", "loop", (HOST_CALLS,))
                    .expect("engine loop"),
            )
        });
    });
    group.finish();
}

fn bench_compute(c: &mut Criterion, backends: &Backends) {
    let Backends { lua, qjs, engine } = backends;
    let mut group = c.benchmark_group("compute_fib20");
    configure(&mut group);
    group.bench_function("mlua", |b| {
        let fib = lua.function("fib");
        b.iter(|| black_box(fib.call::<f64>(FIB_N).expect("lua fib")));
    });
    group.bench_function("quickjs_raw", |b| {
        b.iter(|| {
            qjs.context.with(|ctx| {
                black_box(
                    js_function(&ctx, "fib")
                        .call::<_, f64>((FIB_N,))
                        .expect("quickjs fib"),
                )
            })
        });
    });
    group.bench_function("manivelle_engine", |b| {
        b.iter(|| {
            black_box(
                engine
                    .call::<f64>("bench", "fib", (FIB_N,))
                    .expect("engine fib"),
            )
        });
    });
    group.finish();
}

fn bench_emit(c: &mut Criterion, backends: &Backends) {
    let Backends { lua, qjs, engine } = backends;
    let event = json!({ "combo": 7 });
    let mut group = c.benchmark_group("emit_event_1_listener");
    configure(&mut group);
    group.bench_function("mlua", |b| {
        let handler = lua.function("on_score");
        b.iter(|| {
            let arg = lua.lua.to_value(&event).expect("lua to_value");
            handler.call::<()>(arg).expect("lua handler");
        });
    });
    group.bench_function("quickjs_raw", |b| {
        b.iter(|| {
            qjs.context.with(|ctx| {
                let arg = json_to_js(&ctx, &event).expect("quickjs to js");
                js_function(&ctx, "onScore")
                    .call::<_, ()>((arg,))
                    .expect("quickjs handler");
            })
        });
    });
    group.bench_function("manivelle_engine", |b| {
        b.iter(|| black_box(engine.emit("score.update", &event).expect("engine emit")));
    });
    group.finish();
}

fn bench_reload(c: &mut Criterion, qjs: &QuickJsBench) {
    let mut engine = manivelle_engine();
    let variants = [TS_SOURCE.to_owned(), format!("{TS_SOURCE}\n// variant b\n")];
    let mut group = c.benchmark_group("reload_script_warm");
    configure(&mut group);
    group.bench_function("mlua", |b| {
        let lua = LuaBench::new();
        b.iter(|| lua.lua.load(LUA_SOURCE).exec().expect("lua reload"));
    });
    group.bench_function("quickjs_raw", |b| {
        b.iter(|| {
            let context = Context::full(&qjs.runtime).expect("create context");
            context.with(|ctx| {
                // Same shape as the engine script's `math.inc`, for a fair comparison.
                let math = Object::new(ctx.clone()).expect("create quickjs math");
                math.set("inc", Func::from(|value: f64| value + 1.0))
                    .expect("set quickjs inc");
                ctx.globals().set("math", math).expect("set quickjs math");
                ctx.eval::<(), _>(JS_SOURCE).expect("quickjs reload");
            });
            black_box(context)
        });
    });
    group.bench_function("manivelle_engine", |b| {
        let mut next = 0usize;
        b.iter(|| {
            next ^= 1;
            engine
                .load_script("reload", &variants[next])
                .expect("engine reload");
        });
    });
    group.finish();
}

fn vs_lua_benchmarks(c: &mut Criterion) {
    let backends = Backends {
        lua: LuaBench::new(),
        qjs: QuickJsBench::new(),
        engine: manivelle_engine(),
    };

    bench_call_scalar(c, &backends);
    bench_call_object(c, &backends);
    bench_host_calls(c, &backends);
    bench_compute(c, &backends);
    bench_emit(c, &backends);
    bench_reload(c, &backends.qjs);
}

criterion_group!(benches, vs_lua_benchmarks);
criterion_main!(benches);
