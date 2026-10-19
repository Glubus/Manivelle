//! Engine lifecycle costs: startup, first inline/project load with and without the
//! transpile disk cache, SDK generation, the memory shape of many small scripts, and
//! the per-frame cost of `emit` and `advance_timers` as the script count grows.
//!
//! Per-call and emit overhead are measured against Lua and raw QuickJS in `vs_lua`.

use std::hint::black_box;
use std::path::PathBuf;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use criterion::{
    BatchSize, Bencher, BenchmarkGroup, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use manivelle::{
    Engine, EngineError, EngineOptions, HostCallback, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, TsSchema,
};
use serde_json::json;

#[path = "support/memory.rs"]
mod memory;

use memory::{
    load_many_small_scripts, many_scripts_options, memory_report_child_script_count,
    report_many_small_scripts_process_memory, report_many_small_scripts_process_memory_curve,
};

const BASIC_SCRIPT: &str = include_str!("../tests/projects/basic_math/main.ts");
const MOD_PACK_ENTRY: &str = concat!(
    env!("CARGO_MANIFEST_DIR"),
    "/tests/projects/realistic_mod_pack/src/main.ts"
);
const MANY_SMALL_SCRIPT_COUNT: usize = 400;
/// Scripts per context group in the grouped benches.
const GROUP_SIZE: usize = 10;
const GROUPED_FRAME_SCRIPT: &str = r#"
import { ctx } from "manivelle:env";
let total = 0;
ctx.on("frame", (event: { dt: number }) => { total += event.dt; });
export {};
"#;
const FRAME_SCRIPT_COUNTS: &[usize] = &[1, 50, 250, 500, 1000];
const FRAME_DURATION: Duration = Duration::from_micros(500);
const FRAME_SCRIPT: &str = r#"
let total = 0;
ctx.on("frame", (event: { dt: number }) => { total += event.dt; });
export {};
"#;
/// Listens to an event the benches never emit.
const OTHER_SCRIPT: &str = r#"
let total = 0;
ctx.on("other", () => { total += 1; });
export {};
"#;
/// A zero-delay interval is due on every `advance_timers` call.
const TIMER_SCRIPT: &str = r#"
let total = 0;
setInterval(() => { total += 1; }, 0);
export {};
"#;
/// A timer that is never due: what most frames see, since a timer rarely fires in the
/// 500 µs of a frame.
const IDLE_TIMER_SCRIPT: &str = r#"
setInterval(() => {}, 1e12);
export {};
"#;
const GROUPED_TIMER_SCRIPT: &str = r#"
import { setInterval } from "manivelle:env";
let total = 0;
setInterval(() => { total += 1; }, 0);
export {};
"#;

fn runtime_benchmarks(c: &mut Criterion) {
    if let Some(script_count) = memory_report_child_script_count() {
        report_many_small_scripts_process_memory(script_count);
        std::process::exit(0);
    }

    report_many_small_scripts_process_memory_curve();
    bench_engine_new(c);
    bench_cold_inline_load(c);
    bench_cold_project_load(c);
    bench_sdk_generation(c);
    bench_many_small_scripts_memory_shape(c);
    bench_grouped_load(c);
    bench_frame_budget(c);
}

fn bench_engine_new(c: &mut Criterion) {
    let options = EngineOptions::default();
    c.bench_function("engine_new", |b| {
        b.iter(|| black_box(Engine::new(&options).expect("create engine")));
    });
}

/// One `ctx.on("frame")` handler per script, as in the 500 µs frame (2000 fps) the
/// scripts share with render, input and audio. Throughput is scripts per frame, so
/// the report shows the cost per script and where it stops being linear.
/// `advance_timers` is measured with every script's timer due on each call, and `emit`
/// again with the scripts spread over context groups of `GROUP_SIZE` scripts.
fn bench_frame_budget(c: &mut Criterion) {
    let frame = json!({ "dt": 0.0005, "tick": 1 });
    let mut group = c.benchmark_group("frame_budget");
    configure_slow(&mut group);
    for &script_count in FRAME_SCRIPT_COUNTS {
        let events = scripts_engine(FRAME_SCRIPT, script_count);
        let sparse = sparse_engine(script_count);
        let timers = scripts_engine(TIMER_SCRIPT, script_count);
        let grouped = grouped_engine(GROUPED_FRAME_SCRIPT, script_count);
        let grouped_timers = grouped_engine(GROUPED_TIMER_SCRIPT, script_count);
        let idle_timers = scripts_engine(IDLE_TIMER_SCRIPT, script_count);
        group.throughput(Throughput::Elements(script_count as u64));
        group.bench_function(format!("emit_{script_count}"), |b| {
            b.iter(|| black_box(events.emit("frame", &frame).expect("emit frame")));
        });
        // 1 script in 100 listens to `frame`; the others listen to another event.
        group.bench_function(format!("emit_{script_count}_sparse"), |b| {
            b.iter(|| black_box(sparse.emit("frame", &frame).expect("emit frame")));
        });
        group.bench_function(format!("emit_{script_count}_grouped"), |b| {
            b.iter(|| black_box(grouped.emit("frame", &frame).expect("emit frame")));
        });
        group.bench_function(format!("pump_idle_{script_count}"), |b| {
            b.iter(|| events.pump().expect("pump"));
        });
        group.bench_function(format!("advance_timers_idle_{script_count}"), |b| {
            b.iter(|| {
                black_box(
                    idle_timers
                        .advance_timers(FRAME_DURATION)
                        .expect("advance timers"),
                )
            });
        });
        group.bench_function(format!("advance_timers_{script_count}_grouped"), |b| {
            b.iter(|| {
                black_box(
                    grouped_timers
                        .advance_timers(FRAME_DURATION)
                        .expect("advance timers"),
                )
            });
        });
        group.bench_function(format!("advance_timers_{script_count}"), |b| {
            b.iter(|| {
                black_box(
                    timers
                        .advance_timers(FRAME_DURATION)
                        .expect("advance timers"),
                )
            });
        });
    }
    group.finish();
}

/// `script_count` scripts of which one in 100 listens to `frame`, the rest to `other`.
fn sparse_engine(script_count: usize) -> Engine {
    let mut engine = Engine::new(&many_scripts_options()).expect("create engine");
    for index in 0..script_count {
        let source = if index % 100 == 0 {
            FRAME_SCRIPT
        } else {
            OTHER_SCRIPT
        };
        engine
            .load_script(format!("script-{index}"), source)
            .expect("load script");
    }
    engine
}

fn scripts_engine(source: &str, script_count: usize) -> Engine {
    let mut engine = Engine::new(&many_scripts_options()).expect("create engine");
    for index in 0..script_count {
        engine
            .load_script(format!("script-{index}"), source)
            .expect("load script");
    }
    engine
}

/// [`scripts_engine`] with the scripts loaded into context groups of `GROUP_SIZE`.
fn grouped_engine(source: &str, script_count: usize) -> Engine {
    let mut engine = Engine::new(&many_scripts_options()).expect("create engine");
    for index in 0..script_count {
        engine
            .load_script_in(
                &format!("group-{}", index / GROUP_SIZE),
                format!("script-{index}"),
                source,
            )
            .expect("load script");
    }
    engine
}

/// Loading `MANY_SMALL_SCRIPT_COUNT` scripts into a fresh engine, each in a context of
/// its own and then in groups of `GROUP_SIZE`.
fn bench_grouped_load(c: &mut Criterion) {
    let mut group = c.benchmark_group("load_many_scripts");
    configure_slow(&mut group);
    for (label, size) in [("own_contexts", 1), ("groups", GROUP_SIZE)] {
        group.bench_function(format!("{MANY_SMALL_SCRIPT_COUNT}_{label}"), |b| {
            bench_fresh_engine(
                b,
                || Engine::new(&many_scripts_options()).expect("create engine"),
                |engine| {
                    for index in 0..MANY_SMALL_SCRIPT_COUNT {
                        engine.load_script_in(
                            &format!("group-{}", index / size),
                            format!("script-{index}"),
                            GROUPED_FRAME_SCRIPT,
                        )?;
                    }
                    Ok(())
                },
            );
        });
    }
    group.finish();
}

/// First load of an inline script into a fresh engine: a full transpile without a
/// cache, a disk read of the transpiled artifact with a warm cache.
fn bench_cold_inline_load(c: &mut Criterion) {
    let cache = CacheDir::new("bench-inline-load");
    let cached = cache.options();
    Engine::new(&cached)
        .expect("create engine")
        .load_script("math", BASIC_SCRIPT)
        .expect("warm inline cache");

    let mut group = c.benchmark_group("cold_inline_load");
    for (label, options) in [
        ("no_cache", EngineOptions::default()),
        ("disk_cache", cached),
    ] {
        group.bench_function(label, |b| {
            bench_fresh_engine(
                b,
                || Engine::new(&options).expect("create engine"),
                |engine| engine.load_script("math", BASIC_SCRIPT),
            );
        });
    }
    group.finish();
}

/// First load of the multi-file mod pack (tsconfig aliases, host imports, 8 modules).
fn bench_cold_project_load(c: &mut Criterion) {
    let cache = CacheDir::new("bench-project-load");
    let cached = cache.options();
    mod_pack_engine(&cached)
        .load_project("raid-mod", MOD_PACK_ENTRY)
        .expect("warm project cache");

    let mut group = c.benchmark_group("cold_project_load");
    configure_slow(&mut group);
    for (label, options) in [
        ("no_cache", EngineOptions::default()),
        ("disk_cache", cached),
    ] {
        group.bench_function(label, |b| {
            bench_fresh_engine(
                b,
                || mod_pack_engine(&options),
                |engine| engine.load_project("raid-mod", MOD_PACK_ENTRY),
            );
        });
    }
    group.finish();
}

fn bench_sdk_generation(c: &mut Criterion) {
    let engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function::<BenchFindUser>()
        .and_then(|registry| registry.function::<BenchCreateInvoice>())
        .and_then(|registry| registry.callback::<BenchScoreUpdate>())
        .expect("register host contracts");

    c.bench_function("sdk_generation", |b| {
        b.iter(|| {
            let types = engine.registry().dts().expect("render types");
            let sdk = engine.registry().sdk().expect("render sdk");
            black_box((types, sdk));
        });
    });
}

fn bench_many_small_scripts_memory_shape(c: &mut Criterion) {
    let options = many_scripts_options();
    let mut group = c.benchmark_group("many_small_scripts");
    configure_slow(&mut group);
    group.bench_function(format!("mount_{MANY_SMALL_SCRIPT_COUNT}"), |b| {
        bench_fresh_engine(
            b,
            || Engine::new(&options).expect("create engine"),
            |engine| {
                load_many_small_scripts(engine, MANY_SMALL_SCRIPT_COUNT);
                black_box(engine.memory_stats());
                Ok(())
            },
        );
    });
    group.finish();
}

/// Times `load` on a fresh engine per iteration; engine creation and drop stay
/// outside the measurement.
fn bench_fresh_engine(
    b: &mut Bencher<'_, WallTime>,
    setup: impl Fn() -> Engine,
    load: impl Fn(&mut Engine) -> Result<(), EngineError>,
) {
    b.iter_batched(
        setup,
        |mut engine| {
            load(&mut engine).expect("load script");
            engine
        },
        BatchSize::SmallInput,
    );
}

fn configure_slow(group: &mut BenchmarkGroup<'_, WallTime>) {
    group.sample_size(20);
    group.measurement_time(Duration::from_secs(5));
}

#[derive(TsSchema)]
#[manivelle(name = "CreateInvoiceInput")]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct BenchCreateInvoiceInput {
    account_id: String,
    total: f64,
}

#[derive(TsSchema)]
#[manivelle(name = "CreateInvoiceOutput")]
struct BenchCreateInvoiceOutput {
    id: String,
    accepted: bool,
}

#[derive(TsSchema)]
#[manivelle(name = "ScoreUpdatePayload")]
struct BenchScoreUpdatePayload {
    combo: f64,
}

struct BenchFindUser;
struct BenchCreateInvoice;
struct BenchScoreUpdate;

impl HostContract for BenchFindUser {
    const NAME: &'static str = "user.find";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["user", "find"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for BenchFindUser {
    type Input = u64;
    type Output = String;
}

impl HostFunction for BenchFindUser {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(format!("user-{input}"))
    }
}

impl HostContract for BenchCreateInvoice {
    const NAME: &'static str = "billing.invoice.create";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for BenchCreateInvoice {
    type Input = BenchCreateInvoiceInput;
    type Output = BenchCreateInvoiceOutput;
}

impl HostFunction for BenchCreateInvoice {
    fn call(_input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(BenchCreateInvoiceOutput {
            id: "invoice_1".to_owned(),
            accepted: true,
        })
    }
}

impl HostContract for BenchScoreUpdate {
    const NAME: &'static str = "score.update";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["score", "onUpdate"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for BenchScoreUpdate {
    type Payload = BenchScoreUpdatePayload;
}

/// Engine with the host contracts `realistic_mod_pack` imports from `"test"`.
fn mod_pack_engine(options: &EngineOptions) -> Engine {
    let engine = Engine::new(options).expect("create engine");
    engine
        .registry()
        .function::<BenchFindUser>()
        .and_then(|registry| registry.callback::<BenchScoreUpdate>())
        .expect("register mod pack contracts");
    engine
}

/// Temporary transpile cache directory, removed on drop.
struct CacheDir(PathBuf);

impl CacheDir {
    fn new(label: &str) -> Self {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or(Duration::ZERO)
            .as_nanos();
        Self(std::env::temp_dir().join(format!("manivelle-{label}-{nanos}")))
    }

    fn options(&self) -> EngineOptions {
        EngineOptions {
            cache_dir: Some(self.0.clone()),
            ..EngineOptions::default()
        }
    }
}

impl Drop for CacheDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

criterion_group!(benches, runtime_benchmarks);
criterion_main!(benches);
