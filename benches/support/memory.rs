//! Process memory report for many small scripts: RSS and virtual size from
//! `/proc/self/status` around engine creation, script mount and drop, next to QuickJS's
//! own accounting. Each script count reruns the benchmark binary in a child process so
//! every sample starts from a clean process.

use std::process::{Command, Stdio};

use manivelle::{Engine, EngineOptions, MemoryStats};

const BASIC_SCRIPT: &str = include_str!("../../tests/projects/basic_math/main.ts");
const MEMORY_REPORT_SCRIPT_COUNTS: &[usize] = &[100, 400, 800, 1000];
const MANY_SCRIPTS_MEMORY_LIMIT_BYTES: usize = 256 * 1024 * 1024;
const MEMORY_REPORT_CHILD_ENV: &str = "MANIVELLE_MEMORY_REPORT_SCRIPT_COUNT";
const BYTES_PER_KIB: f64 = 1024.0;
const BYTES_PER_MIB: f64 = 1024.0 * 1024.0;

pub fn report_many_small_scripts_process_memory_curve() {
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

pub fn memory_report_child_script_count() -> Option<usize> {
    std::env::var(MEMORY_REPORT_CHILD_ENV)
        .ok()?
        .parse::<usize>()
        .ok()
        .filter(|script_count| *script_count > 0)
}

pub fn report_many_small_scripts_process_memory(script_count: usize) {
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

pub fn load_many_small_scripts(engine: &mut Engine, script_count: usize) {
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

pub fn many_scripts_options() -> EngineOptions {
    EngineOptions {
        memory_limit_bytes: MANY_SCRIPTS_MEMORY_LIMIT_BYTES,
        ..EngineOptions::default()
    }
}
