//! QuickJS memory usage conversion.

use rquickjs::runtime::MemoryUsage;

use crate::types::MemoryStats;

pub(crate) fn memory_stats(usage: MemoryUsage) -> MemoryStats {
    MemoryStats {
        malloc_size_bytes: non_negative(usage.malloc_size),
        malloc_limit_bytes: non_negative(usage.malloc_limit),
        memory_used_bytes: non_negative(usage.memory_used_size),
        malloc_count: non_negative(usage.malloc_count),
        atom_count: non_negative(usage.atom_count),
        string_count: non_negative(usage.str_count),
        object_count: non_negative(usage.obj_count),
        function_count: non_negative(usage.js_func_count),
    }
}

/// QuickJS reports its counts as non-negative; a negative value would be a bug there, and
/// it is reported as zero rather than as a huge unsigned count.
fn non_negative(value: i64) -> u64 {
    u64::try_from(value).unwrap_or_default()
}
