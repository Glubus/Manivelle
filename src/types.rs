//! Shared public types.

use serde::{Deserialize, Serialize};

use crate::error::EngineError;

/// Stable identifier for a loaded script.
pub type ScriptId = String;

/// QuickJS memory counters for one [`Engine`](crate::Engine).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[non_exhaustive]
pub struct MemoryStats {
    /// Total bytes allocated by the QuickJS allocator.
    pub malloc_size_bytes: u64,
    /// QuickJS allocator limit in bytes, or zero when unlimited.
    pub malloc_limit_bytes: u64,
    /// Bytes currently used by live QuickJS values and runtime structures.
    pub memory_used_bytes: u64,
    /// Allocation count reported by QuickJS.
    pub malloc_count: u64,
    /// Live atom count reported by QuickJS.
    pub atom_count: u64,
    /// Live string count reported by QuickJS.
    pub string_count: u64,
    /// Live object count reported by QuickJS.
    pub object_count: u64,
    /// JavaScript function count reported by QuickJS.
    pub function_count: u64,
}

/// Outcome of [`Engine::reload_changed`](crate::Engine::reload_changed).
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct ReloadReport {
    /// Scripts reloaded because their files changed, in load order.
    pub reloaded: Vec<ScriptId>,
    /// Scripts whose files changed but whose reload failed; each keeps running its
    /// previous version.
    pub failed: Vec<(ScriptId, EngineError)>,
    /// Scripts reloaded, and so also listed in `reloaded`, whose previous version's
    /// `ctx.hot.dispose` callback threw after the new version was loaded.
    pub dispose_failed: Vec<(ScriptId, EngineError)>,
}
