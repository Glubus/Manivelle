//! Shared public types.

use std::borrow::Borrow;
use std::fmt;
use std::ops::Deref;
use std::sync::Arc;

use serde::{Deserialize, Serialize};

use crate::error::EngineError;

/// Stable identifier of a loaded script: the name the host application gave it in
/// [`Engine::load_script`](crate::Engine::load_script) or
/// [`Engine::load_project`](crate::Engine::load_project).
///
/// Cloning shares the text instead of copying it. A `&str` finds the script in a map keyed
/// by `ScriptId` without allocating, through `Borrow<str>`.
#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ScriptId(Arc<str>);

impl ScriptId {
    /// The id as text.
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Deref for ScriptId {
    type Target = str;

    fn deref(&self) -> &str {
        &self.0
    }
}

impl AsRef<str> for ScriptId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

impl Borrow<str> for ScriptId {
    fn borrow(&self) -> &str {
        &self.0
    }
}

impl fmt::Display for ScriptId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(&self.0, f)
    }
}

impl From<&str> for ScriptId {
    fn from(id: &str) -> Self {
        Self(Arc::from(id))
    }
}

impl From<String> for ScriptId {
    fn from(id: String) -> Self {
        Self(Arc::from(id))
    }
}

impl PartialEq<str> for ScriptId {
    fn eq(&self, other: &str) -> bool {
        &*self.0 == other
    }
}

impl PartialEq<&str> for ScriptId {
    fn eq(&self, other: &&str) -> bool {
        &*self.0 == *other
    }
}

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

/// Outcome of [`Engine::emit_report`](crate::Engine::emit_report): what an event did to
/// each script.
#[derive(Debug, Default)]
#[non_exhaustive]
pub struct Emission {
    /// How many scripts had at least one handler for the event.
    pub delivered: usize,
    /// Each script whose handlers failed, in delivery order, with its first error: a
    /// throw, a rejection, or a script that ran out of its time or memory budget. Every
    /// other handler ran regardless.
    pub failures: Vec<(ScriptId, EngineError)>,
    /// An error no script owns: a Promise job queued by a handler that threw later, an
    /// unhandled rejection, or the host's interrupt.
    pub unattributed: Option<EngineError>,
}
