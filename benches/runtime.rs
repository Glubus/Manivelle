//! Engine lifecycle costs: startup, first inline/project load with and without the
//! transpile disk cache, SDK generation, the memory shape of many small scripts, and
//! the per-frame cost of `emit` and `advance_timers` as the script count grows.
//!
//! Per-call and emit overhead are measured against Lua and raw QuickJS in `vs_lua`.

use std::hint::black_box;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use criterion::{
    BatchSize, Bencher, BenchmarkGroup, Criterion, Throughput, criterion_group, criterion_main,
    measurement::WallTime,
};
use manivelle::{
    Engine, HostCallback, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
    MemoryStats, TsSchema, VmError, VmOptions,
};
use serde_json::json;

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
const MEMORY_REPORT_SCRIPT_COUNTS: &[usize] = &[100, 400, 800, 1000];
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
const MANY_SCRIPTS_MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
const MEMORY_REPORT_CHILD_ENV: &str = "MANIVELLE_MEMORY_REPORT_SCRIPT_COUNT";
const BYTES_PER_KIB: f64 = 1024.0;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

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
    let options = VmOptions::default();
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
    for (label, options) in [("no_cache", VmOptions::default()), ("disk_cache", cached)] {
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
    for (label, options) in [("no_cache", VmOptions::default()), ("disk_cache", cached)] {
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
    let engine = Engine::new(&VmOptions::default()).expect("create engine");
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
    load: impl Fn(&mut Engine) -> Result<(), VmError>,
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

fn report_many_small_scripts_process_memory_curve() {
    eprintln!();
    eprintln!("memory_rss_small_scripts_curve:");
    for &script_count in MEMORY_REPORT_SCRIPT_COUNTS {
        run_isolated_process_memory_report(script_count);
    }
    eprintln!();
}

/// Reruns this benchmark binary with one script count so every RSS sample starts
/// from a clean process.
fn run_isolated_process_memory_report(script_count: usize) {
    let current_exe = std::env::current_exe().expect("resolve current benchmark executable");
    let status = Command::new(current_exe)
        .env(MEMORY_REPORT_CHILD_ENV, script_count.to_string())
        .stdout(Stdio::inherit())
        .stderr(Stdio::inherit())
        .status()
        .expect("run isolated memory benchmark process");
    assert!(
        status.success(),
        "isolated memory benchmark failed for {script_count} scripts: {status}"
    );
}

fn memory_report_child_script_count() -> Option<usize> {
    std::env::var(MEMORY_REPORT_CHILD_ENV)
        .ok()?
        .parse::<usize>()
        .ok()
        .filter(|script_count| *script_count > 0)
}

fn report_many_small_scripts_process_memory(script_count: usize) {
    let before = read_current_process_memory();
    let mut engine = Engine::new(&many_scripts_options()).expect("create memory report engine");
    let after_engine_new = read_current_process_memory();

    load_many_small_scripts(&mut engine, script_count);
    let after_mount = read_current_process_memory();
    let quickjs = engine.memory_stats();

    drop(engine);
    let after_drop = read_current_process_memory();

    print_process_memory_report(&ProcessMemoryReport {
        script_count,
        before,
        after_engine_new,
        after_mount,
        after_drop,
        quickjs,
    });
}

fn load_many_small_scripts(engine: &mut Engine, script_count: usize) {
    for index in 0..script_count {
        engine
            .load_script(format!("script-{index}"), BASIC_SCRIPT)
            .expect("load small script");
    }
}

#[derive(Debug, Clone, Copy)]
struct ProcessMemory {
    resident_bytes: u64,
    virtual_bytes: u64,
}

#[derive(Debug, Clone, Copy)]
struct ProcessMemoryReport {
    script_count: usize,
    before: Option<ProcessMemory>,
    after_engine_new: Option<ProcessMemory>,
    after_mount: Option<ProcessMemory>,
    after_drop: Option<ProcessMemory>,
    quickjs: MemoryStats,
}

fn print_process_memory_report(report: &ProcessMemoryReport) {
    eprintln!();
    eprintln!("memory_rss_{}_small_scripts:", report.script_count);
    eprintln!(
        "  config: quickjs_memory_limit={:.2} MiB",
        bytes_to_mib(MANY_SCRIPTS_MEMORY_LIMIT_BYTES as u64)
    );
    print_process_memory_snapshot("before", report.before);
    print_process_memory_snapshot("after_engine_new", report.after_engine_new);
    print_process_memory_snapshot("after_mount", report.after_mount);
    print_process_memory_snapshot("after_drop", report.after_drop);
    print_process_memory_delta("engine_new_delta", report.before, report.after_engine_new);
    print_process_memory_delta("mount_delta", report.after_engine_new, report.after_mount);
    print_process_memory_delta("total_delta", report.before, report.after_mount);
    print_quickjs_memory(report.script_count, report.quickjs);
    print_per_script_delta(
        report.script_count,
        report.after_engine_new,
        report.after_mount,
    );
    eprintln!();
}

fn print_quickjs_memory(script_count: usize, stats: MemoryStats) {
    eprintln!(
        "  quickjs_used: {:.2} MiB ({}), objects={}, functions={}",
        bytes_to_mib(stats.memory_used_bytes),
        format_pressure(stats.memory_used_bytes, stats.malloc_limit_bytes),
        stats.object_count,
        stats.function_count
    );
    eprintln!(
        "  quickjs_used_per_script: {:.2} KiB",
        stats.memory_used_bytes as f64 / script_count as f64 / BYTES_PER_KIB
    );
}

fn print_process_memory_snapshot(label: &str, snapshot: Option<ProcessMemory>) {
    match snapshot {
        Some(stats) => {
            eprintln!(
                "  {label}: rss={:.2} MiB, virtual={:.2} MiB",
                bytes_to_mib(stats.resident_bytes),
                bytes_to_mib(stats.virtual_bytes)
            );
        }
        None => {
            eprintln!("  {label}: unavailable");
        }
    }
}

fn print_process_memory_delta(
    label: &str,
    before: Option<ProcessMemory>,
    after: Option<ProcessMemory>,
) {
    match (before, after) {
        (Some(before), Some(after)) => {
            let resident_delta = byte_delta(after.resident_bytes, before.resident_bytes);
            let virtual_delta = byte_delta(after.virtual_bytes, before.virtual_bytes);
            eprintln!(
                "  {label}: rss={}, virtual={}",
                format_mib_delta(resident_delta),
                format_mib_delta(virtual_delta)
            );
        }
        _ => {
            eprintln!("  {label}: unavailable");
        }
    }
}

fn print_per_script_delta(
    script_count: usize,
    before: Option<ProcessMemory>,
    after: Option<ProcessMemory>,
) {
    match (script_count, before, after) {
        (0, _, _) | (_, None, _) | (_, _, None) => {
            eprintln!("  mount_rss_per_script: unavailable");
        }
        (script_count, Some(before), Some(after)) => {
            let resident_delta = byte_delta(after.resident_bytes, before.resident_bytes);
            eprintln!(
                "  mount_rss_per_script: {}",
                format_kib_per_item(resident_delta, script_count)
            );
        }
    }
}

fn read_current_process_memory() -> Option<ProcessMemory> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    Some(ProcessMemory {
        resident_bytes: parse_status_kib_value(&status, "VmRSS:")?,
        virtual_bytes: parse_status_kib_value(&status, "VmSize:")?,
    })
}

fn parse_status_kib_value(status: &str, key: &str) -> Option<u64> {
    let line = status.lines().find(|line| line.starts_with(key))?;
    let value_kib = line.split_whitespace().nth(1)?.parse::<u64>().ok()?;
    Some(value_kib * 1024)
}

fn byte_delta(after: u64, before: u64) -> i128 {
    i128::from(after) - i128::from(before)
}

fn bytes_to_mib(bytes: u64) -> f64 {
    bytes as f64 / BYTES_PER_MIB
}

fn format_mib_delta(delta: i128) -> String {
    format_signed_float(delta as f64 / BYTES_PER_MIB, "MiB")
}

fn format_kib_per_item(delta: i128, item_count: usize) -> String {
    let per_item = delta as f64 / item_count as f64 / BYTES_PER_KIB;
    format_signed_float(per_item, "KiB")
}

fn format_pressure(used_bytes: u64, limit_bytes: u64) -> String {
    if limit_bytes == 0 {
        return String::from("unlimited");
    }

    format!(
        "{:.2}% of limit",
        used_bytes as f64 / limit_bytes as f64 * 100.0
    )
}

fn format_signed_float(value: f64, unit: &str) -> String {
    format!("{value:+.2} {unit}")
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
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
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
    fn call(_input: Self::Input) -> Result<Self::Output, VmError> {
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
fn mod_pack_engine(options: &VmOptions) -> Engine {
    let engine = Engine::new(options).expect("create engine");
    engine
        .registry()
        .function::<BenchFindUser>()
        .and_then(|registry| registry.callback::<BenchScoreUpdate>())
        .expect("register mod pack contracts");
    engine
}

fn many_scripts_options() -> VmOptions {
    VmOptions {
        memory_limit_bytes: MANY_SCRIPTS_MEMORY_LIMIT_BYTES,
        ..VmOptions::default()
    }
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

    fn options(&self) -> VmOptions {
        VmOptions {
            cache_dir: Some(self.0.clone()),
            ..VmOptions::default()
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
