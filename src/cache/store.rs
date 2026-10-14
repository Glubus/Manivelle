//! Disk-backed cache store for transpiled JavaScript and its source map.

use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use crate::compiler::{SourceMap, TranspiledModule};
use crate::error::VmError;

/// First line of an artifact, before the BLAKE3 hash of the rest. The rest is the
/// source map `mappings` line, then the JavaScript.
const ARTIFACT_HEADER: &str = "// manivelle-cache-v3 ";

#[derive(Debug, Clone)]
pub(crate) struct ScriptCache {
    root: PathBuf,
}

impl ScriptCache {
    pub(crate) fn new(root: impl Into<PathBuf>) -> Result<Self, VmError> {
        let root = root.into();
        fs::create_dir_all(&root)?;
        Ok(Self { root })
    }

    /// The artifact stored under `cache_key`, if one can be read and verifies. A cache
    /// only saves work, so an artifact that cannot be read for any reason (missing,
    /// corrupt, unreadable) is a miss rather than an error.
    pub(crate) fn load(&self, cache_key: &str) -> Option<TranspiledModule> {
        let artifact = fs::read_to_string(self.js_path(cache_key)).ok()?;
        verified_payload(&artifact).and_then(transpiled_module)
    }

    pub(crate) fn store(&self, cache_key: &str, module: &TranspiledModule) -> Result<(), VmError> {
        let payload = artifact_payload(module);
        let mut temporary = tempfile::NamedTempFile::new_in(&self.root)?;
        writeln!(
            temporary,
            "{ARTIFACT_HEADER}{}",
            blake3::hash(payload.as_bytes()).to_hex()
        )?;
        temporary.write_all(payload.as_bytes())?;
        temporary.as_file().sync_all()?;
        persist_atomic(temporary, &self.js_path(cache_key))?;
        Ok(())
    }

    fn js_path(&self, cache_key: &str) -> PathBuf {
        self.root.join(cache_file_name(cache_key))
    }
}

fn verified_payload(artifact: &str) -> Option<&str> {
    let (header, payload) = artifact.split_once('\n')?;
    let expected = header.strip_prefix(ARTIFACT_HEADER)?;
    (blake3::hash(payload.as_bytes()).to_hex().as_str() == expected).then_some(payload)
}

/// The source map `mappings` line, then the JavaScript.
fn artifact_payload(module: &TranspiledModule) -> String {
    let mut payload = module.source_map.encode();
    payload.push('\n');
    payload.push_str(&module.js);
    payload
}

/// Splits a verified payload into its source map and JavaScript.
fn transpiled_module(payload: &str) -> Option<TranspiledModule> {
    let (mappings, js) = payload.split_once('\n')?;
    Some(TranspiledModule {
        js: js.to_owned(),
        source_map: Arc::new(SourceMap::decode(mappings)?),
    })
}

fn persist_atomic(
    mut temporary: tempfile::NamedTempFile,
    path: &std::path::Path,
) -> std::io::Result<()> {
    // Windows can transiently deny replacement while another writer/AV has the
    // destination open. Retry the atomic rename; never delete the destination.
    for attempt in 0..50 {
        match temporary.persist(path) {
            Ok(_) => return Ok(()),
            Err(error)
                if cfg!(windows)
                    && error.error.kind() == std::io::ErrorKind::PermissionDenied
                    && attempt < 49 =>
            {
                temporary = error.file;
                std::thread::sleep(std::time::Duration::from_millis(2));
            }
            Err(error) => return Err(error.error),
        }
    }
    unreachable!("final attempt returns its error")
}

fn cache_file_name(cache_key: &str) -> String {
    let mut file_name = String::with_capacity(cache_key.len() + 3);
    file_name.push_str(cache_key);
    file_name.push_str(".js");
    file_name
}

#[cfg(test)]
mod tests {
    use super::*;

    fn module(js: &str) -> TranspiledModule {
        TranspiledModule {
            js: js.to_owned(),
            source_map: Arc::default(),
        }
    }

    #[test]
    fn concurrent_readers_only_see_complete_artifacts() {
        let root = tempfile::tempdir().unwrap();
        let cache = ScriptCache::new(root.path()).unwrap();
        let first = module(&"a".repeat(64 * 1024));
        let second = module(&"b".repeat(64 * 1024));
        cache.store("shared", &first).unwrap();
        std::thread::scope(|scope| {
            for artifact in [&first, &second] {
                let cache = &cache;
                scope.spawn(move || {
                    for _ in 0..30 {
                        cache.store("shared", artifact).unwrap();
                    }
                });
            }
            for _ in 0..100 {
                let loaded = cache.load("shared").expect("complete artifact");
                assert!(loaded == first || loaded == second);
            }
        });
    }

    #[test]
    fn corrupt_and_legacy_entries_are_cache_misses() {
        let root = tempfile::tempdir().unwrap();
        let cache = ScriptCache::new(root.path()).unwrap();
        let legacy_js = "export const answer = 42;";
        let legacy_v2 = format!(
            "// rustts-cache-v2 {}\n{legacy_js}",
            blake3::hash(legacy_js.as_bytes()).to_hex()
        );
        for content in [
            "legacy",
            "// manivelle-cache-v3 invalid\npartial",
            legacy_v2.as_str(),
        ] {
            fs::write(cache.js_path("broken"), content).unwrap();
            assert_eq!(cache.load("broken"), None);
        }
        cache.store("broken", &module(legacy_js)).unwrap();
        assert_eq!(cache.load("broken").unwrap(), module(legacy_js));
    }
}
