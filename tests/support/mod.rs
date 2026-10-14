use std::fs;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{SystemTime, UNIX_EPOCH};

use manivelle::VmOptions;

#[allow(dead_code)]
pub(crate) fn run_tsc(path: &std::path::Path) -> Option<std::process::Output> {
    let compiler =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("node_modules/typescript/bin/tsc");
    let required =
        std::env::var_os("CI").is_some() || std::env::var_os("MANIVELLE_REQUIRE_TSC").is_some();
    if !compiler.is_file() {
        assert!(!required, "TypeScript is required in CI: run npm ci first");
        eprintln!("TypeScript check skipped locally: run npm ci to enable it");
        return None;
    }
    Some(
        std::process::Command::new("node")
            .arg(compiler)
            .args(["--noEmit", "--target", "ES2020", "--module", "ES2020"])
            .arg(path)
            .output()
            .expect("could not execute the pinned TypeScript compiler with node"),
    )
}

static NEXT_TEST_ID: AtomicU64 = AtomicU64::new(1);

pub(crate) struct TestCacheDir {
    path: PathBuf,
}

impl TestCacheDir {
    pub(crate) fn new(prefix: &str) -> Self {
        let path = unique_path(prefix);
        fs::create_dir_all(&path).expect("create test cache dir");
        Self { path }
    }

    /// Engine options caching transpiled artifacts in [`TestCacheDir::cache_path`].
    #[allow(dead_code)]
    pub(crate) fn engine_options(&self) -> VmOptions {
        VmOptions {
            cache_dir: Some(self.cache_path()),
            ..VmOptions::default()
        }
    }

    /// Transpile cache directory, kept apart from project files written under `path()`.
    #[allow(dead_code)]
    pub(crate) fn cache_path(&self) -> PathBuf {
        self.path.join("cache")
    }

    #[allow(dead_code)]
    pub(crate) fn path(&self) -> &PathBuf {
        &self.path
    }
}

impl Drop for TestCacheDir {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

fn unique_path(prefix: &str) -> PathBuf {
    let sequence = NEXT_TEST_ID.fetch_add(1, Ordering::Relaxed);
    let timestamp = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system time before epoch")
        .as_nanos();
    std::env::temp_dir().join(format!(
        "manivelle-{prefix}-{}-{timestamp}-{sequence}",
        std::process::id()
    ))
}
