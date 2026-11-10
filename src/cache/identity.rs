//! Versioned cache keys for transpiled modules.

/// Bump when the artifact layout or the transpile settings change.
const CACHE_SCHEMA_VERSION: &str = "3";
const OXC_VERSION: &str = "0.137.0";
/// What the transpiler is asked for (see `CompilerService`): ES modules, with a source map.
/// Change it when those options change.
const TRANSPILE_OPTIONS: &str = "esm;sourcemap";

/// Key of one transpiled module. Transpiling depends only on the source text, its
/// source type (the file extension) and the compiler, never on import resolution, so
/// every module of every project shares one content-addressed cache.
pub(crate) fn module_cache_key(source: &str, source_type: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    for part in [
        CACHE_SCHEMA_VERSION,
        env!("CARGO_PKG_VERSION"),
        OXC_VERSION,
        TRANSPILE_OPTIONS,
        source_type,
    ] {
        hasher.update(part.as_bytes());
        hasher.update(&[0]);
    }
    hasher.update(source.as_bytes());
    hasher.finalize().to_hex().to_string()
}

#[cfg(test)]
mod tests {
    use super::{OXC_VERSION, module_cache_key};

    #[test]
    fn same_source_with_another_source_type_gets_another_key() {
        let source = "export const view = <div />;";

        assert_ne!(
            module_cache_key(source, "ts"),
            module_cache_key(source, "tsx")
        );
    }

    #[test]
    fn the_oxc_version_of_the_key_is_the_one_cargo_builds() {
        let manifest = include_str!("../../Cargo.toml");

        assert!(
            manifest.contains(&format!("oxc = {{ version = \"{OXC_VERSION}\"")),
            "OXC_VERSION ({OXC_VERSION}) is not the version of `oxc` in Cargo.toml: bump both"
        );
    }

    #[test]
    fn a_changed_source_gets_another_key() {
        assert_ne!(
            module_cache_key("export const a = 1;", "ts"),
            module_cache_key("export const a = 2;", "ts")
        );
    }

    #[test]
    fn the_same_source_and_type_always_get_the_same_key() {
        let source = "export const a = 1;";

        assert_eq!(
            module_cache_key(source, "ts"),
            module_cache_key(source, "ts")
        );
    }
}
