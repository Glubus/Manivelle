//! Project module resolution backed by `oxc_resolver`.

use std::fs;
use std::path::{Path, PathBuf};

use oxc_resolver::{
    ExtendsField, ResolveOptions, Resolver, TsConfig, TsconfigDiscovery, TsconfigOptions,
    TsconfigReferences,
};

use crate::error::VmError;

const MODULE_EXTENSIONS: &[&str] = &[".ts", ".tsx", ".mts", ".js", ".jsx", ".mjs"];

#[derive(Debug)]
pub(crate) struct ModuleResolver {
    resolver: Resolver,
    project_root: PathBuf,
    /// The `tsconfig.json` found for the entry, then the configs it `extends`,
    /// directly or not.
    tsconfig_files: Vec<PathBuf>,
}

#[derive(Debug)]
/// Canonical path of a resolved module, and the `package.json` that applied to it.
pub(crate) struct ResolvedModule {
    pub(crate) path: PathBuf,
    pub(crate) package_json: Option<PathBuf>,
}

impl ModuleResolver {
    pub(crate) fn for_entry(entry_path: &Path) -> Result<Self, VmError> {
        let entry_dir = entry_path.parent().ok_or_else(|| VmError::Resolve {
            details: format!("entry has no parent directory: {}", entry_path.display()),
        })?;
        let tsconfig_path = find_tsconfig(entry_dir);
        let project_root = project_root(entry_dir, tsconfig_path.as_deref())?;
        let resolver = Resolver::new(resolve_options(tsconfig_path.as_deref()));
        let tsconfig_files = tsconfig_path
            .as_deref()
            .map(tsconfig_chain)
            .unwrap_or_default();

        Ok(Self {
            resolver,
            project_root,
            tsconfig_files,
        })
    }

    /// The `tsconfig.json` this project resolves `paths` and `baseUrl` with, then the
    /// configs it `extends` through a relative or absolute path, at any depth: what a
    /// change to the project's resolution settings shows up in.
    pub(crate) fn tsconfig_files(&self) -> &[PathBuf] {
        &self.tsconfig_files
    }

    /// Canonical root of the project: the `tsconfig.json` directory, else the entry's.
    pub(crate) fn project_root(&self) -> &Path {
        &self.project_root
    }

    pub(crate) fn resolve_request(
        &self,
        from_path: &Path,
        request: &str,
    ) -> Result<ResolvedModule, VmError> {
        let base_dir = parent_dir(from_path)?;
        let resolution =
            self.resolver
                .resolve(base_dir, request)
                .map_err(|error| VmError::Resolve {
                    details: format!(
                        "unable to resolve import `{request}` from {}: {error}",
                        from_path.display()
                    ),
                })?;
        let path = self.canonical_in_project(request, resolution.path())?;

        Ok(ResolvedModule {
            path,
            package_json: resolution
                .package_json()
                .map(|package_json| package_json.path().to_path_buf()),
        })
    }

    fn canonical_in_project(
        &self,
        request: &str,
        resolved_path: &Path,
    ) -> Result<PathBuf, VmError> {
        let canonical_resolved_path = resolved_path.canonicalize().map_err(VmError::from)?;
        if canonical_resolved_path.starts_with(&self.project_root) {
            return Ok(canonical_resolved_path);
        }

        Err(VmError::Resolve {
            details: format!(
                "resolved import `{request}` outside project root {}: {}",
                self.project_root.display(),
                canonical_resolved_path.display()
            ),
        })
    }
}

fn resolve_options(tsconfig_path: Option<&Path>) -> ResolveOptions {
    ResolveOptions {
        tsconfig: tsconfig_path.map(|config_file| {
            TsconfigDiscovery::Manual(TsconfigOptions {
                config_file: config_file.to_path_buf(),
                references: TsconfigReferences::Disabled,
            })
        }),
        extensions: MODULE_EXTENSIONS
            .iter()
            .map(|extension| (*extension).to_owned())
            .collect(),
        main_files: vec![String::from("index")],
        // Scripts are ES modules: a package's `exports` resolve through the conditions
        // of an `import` (never `require` or `node`, whose targets may be CommonJS),
        // and a package without `exports` through its `module` entry before `main`.
        condition_names: ["import", "module", "default"]
            .into_iter()
            .map(String::from)
            .collect(),
        main_fields: ["module", "main"].into_iter().map(String::from).collect(),
        modules: vec![String::from("node_modules")],
        node_path: false,
        builtin_modules: false,
        ..ResolveOptions::default()
    }
}

fn parent_dir(path: &Path) -> Result<&Path, VmError> {
    path.parent().ok_or_else(|| VmError::Resolve {
        details: format!("module has no parent directory: {}", path.display()),
    })
}

fn find_tsconfig(start_dir: &Path) -> Option<PathBuf> {
    start_dir
        .ancestors()
        .map(|dir| dir.join("tsconfig.json"))
        .find(|candidate| candidate.is_file())
}

/// `tsconfig` and the configs it extends, following `extends` the way the resolver
/// does for relative and absolute paths. Configs taken from packages (`"@scope/base"`,
/// `"#config"`) live in `node_modules`, which is not watched, and are not followed. A
/// config that cannot be read or parsed ends its branch; it is listed all the same, so
/// fixing it shows up as a change.
fn tsconfig_chain(tsconfig: &Path) -> Vec<PathBuf> {
    let mut files = Vec::new();
    let mut pending = vec![tsconfig.to_path_buf()];
    while let Some(path) = pending.pop() {
        if files.contains(&path) {
            continue;
        }
        let extends = fs::read_to_string(&path)
            .ok()
            .and_then(|json| TsConfig::parse(false, &path, &path, json).ok())
            .and_then(|config| config.extends);
        let directory = path.parent().map(Path::to_path_buf).unwrap_or_default();
        files.push(path);
        let specifiers = match &extends {
            Some(ExtendsField::Single(specifier)) => std::slice::from_ref(specifier),
            Some(ExtendsField::Multiple(specifiers)) => specifiers.as_slice(),
            None => &[],
        };
        pending.extend(
            specifiers
                .iter()
                .filter(|specifier| {
                    specifier.starts_with('.') || Path::new(specifier).is_absolute()
                })
                .map(|specifier| extended_config_file(&directory.join(specifier))),
        );
    }
    files
}

/// The file an `extends` path names: the file itself, a directory's `tsconfig.json`,
/// else the path with `.json` appended.
fn extended_config_file(path: &Path) -> PathBuf {
    if path.is_file() {
        return path.to_path_buf();
    }
    if path.is_dir() {
        return path.join("tsconfig.json");
    }
    let mut file = path.as_os_str().to_owned();
    file.push(".json");
    PathBuf::from(file)
}

fn project_root(entry_dir: &Path, tsconfig_path: Option<&Path>) -> Result<PathBuf, VmError> {
    let root = match tsconfig_path {
        Some(path) => parent_dir(path)?,
        None => entry_dir,
    };

    root.canonicalize().map_err(VmError::from)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn resolves_extensionless_relative_module() {
        let root = temp_fixture_dir("extensionless");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("create fixture dir");
        fs::write(root.join("main.ts"), "").expect("write main");
        fs::write(src.join("math.ts"), "").expect("write module");

        let resolver = ModuleResolver::for_entry(&root.join("main.ts")).expect("create resolver");
        let resolved = resolver
            .resolve_request(&root.join("main.ts"), "./src/math")
            .expect("resolve module");

        assert_eq!(
            resolved.path,
            src.join("math.ts")
                .canonicalize()
                .expect("canonical fixture path")
        );
    }

    #[test]
    fn resolves_index_module() {
        let root = temp_fixture_dir("index");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("create fixture dir");
        fs::write(root.join("main.ts"), "").expect("write main");
        fs::write(src.join("index.ts"), "").expect("write module");

        let resolver = ModuleResolver::for_entry(&root.join("main.ts")).expect("create resolver");
        let resolved = resolver
            .resolve_request(&root.join("main.ts"), "./src")
            .expect("resolve index module");

        assert_eq!(
            resolved.path,
            src.join("index.ts")
                .canonicalize()
                .expect("canonical fixture path")
        );
    }

    #[test]
    fn rejects_unresolved_package_requests() {
        let root = temp_fixture_dir("missing-package");
        fs::create_dir_all(&root).expect("create fixture dir");
        fs::write(root.join("main.ts"), "").expect("write main");
        let resolver = ModuleResolver::for_entry(&root.join("main.ts")).expect("create resolver");
        let error = resolver
            .resolve_request(&root.join("main.ts"), "missing-pkg")
            .expect_err("reject unresolved package import");

        assert!(matches!(error, VmError::Resolve { .. }));
    }

    #[test]
    fn resolves_package_import_from_local_node_modules() {
        let root = temp_fixture_dir("package-import");
        let package = root.join("node_modules").join("demo-pkg");
        fs::create_dir_all(&package).expect("create fixture dir");
        fs::write(root.join("main.ts"), "").expect("write main");
        fs::write(
            package.join("package.json"),
            r#"{"name":"demo-pkg","main":"index.ts"}"#,
        )
        .expect("write package manifest");
        fs::write(package.join("index.ts"), "").expect("write package entry");

        let resolver = ModuleResolver::for_entry(&root.join("main.ts")).expect("create resolver");
        let resolved = resolver
            .resolve_request(&root.join("main.ts"), "demo-pkg")
            .expect("resolve package import");

        assert_eq!(
            resolved.path,
            package
                .join("index.ts")
                .canonicalize()
                .expect("canonical fixture path")
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_package_import_symlinked_outside_project_root() {
        use std::os::unix::fs::symlink;

        let root = temp_fixture_dir("external-package-symlink");
        let external = temp_fixture_dir("external-package-target");
        let node_modules = root.join("node_modules");
        let package_link = node_modules.join("demo-pkg");
        fs::create_dir_all(&node_modules).expect("create node_modules");
        fs::create_dir_all(&external).expect("create external package");
        fs::write(root.join("main.ts"), "").expect("write main");
        fs::write(
            external.join("package.json"),
            r#"{"name":"demo-pkg","main":"index.ts"}"#,
        )
        .expect("write package manifest");
        fs::write(external.join("index.ts"), "").expect("write package entry");
        symlink(&external, &package_link).expect("symlink external package");

        let resolver = ModuleResolver::for_entry(&root.join("main.ts")).expect("create resolver");
        let error = resolver
            .resolve_request(&root.join("main.ts"), "demo-pkg")
            .expect_err("reject external package symlink");

        assert!(matches!(
            error,
            VmError::Resolve { details }
                if details.contains("outside project root")
        ));
    }

    #[test]
    fn resolves_tsconfig_paths_alias() {
        let root = temp_fixture_dir("tsconfig-alias");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("create fixture dir");
        fs::write(
            root.join("tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "baseUrl": ".",
                    "paths": {
                        "@app/*": ["src/*"]
                    }
                }
            }"#,
        )
        .expect("write tsconfig");
        fs::write(root.join("main.ts"), "").expect("write main");
        fs::write(src.join("math.ts"), "").expect("write module");

        let resolver = ModuleResolver::for_entry(&root.join("main.ts")).expect("create resolver");
        let resolved = resolver
            .resolve_request(&root.join("main.ts"), "@app/math")
            .expect("resolve aliased module");

        assert_eq!(
            resolved.path,
            src.join("math.ts")
                .canonicalize()
                .expect("canonical fixture path")
        );
    }

    #[test]
    fn resolves_tsconfig_base_url_request() {
        let root = temp_fixture_dir("tsconfig-base-url");
        let src = root.join("src");
        fs::create_dir_all(&src).expect("create fixture dir");
        fs::write(
            root.join("tsconfig.json"),
            r#"{
                "compilerOptions": {
                    "baseUrl": "."
                }
            }"#,
        )
        .expect("write tsconfig");
        fs::write(root.join("main.ts"), "").expect("write main");
        fs::write(src.join("math.ts"), "").expect("write module");

        let resolver = ModuleResolver::for_entry(&root.join("main.ts")).expect("create resolver");
        let resolved = resolver
            .resolve_request(&root.join("main.ts"), "src/math")
            .expect("resolve baseUrl module");

        assert_eq!(
            resolved.path,
            src.join("math.ts")
                .canonicalize()
                .expect("canonical fixture path")
        );
    }

    fn temp_fixture_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock before unix epoch")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "manivelle-resolver-{name}-{}-{nanos}",
            std::process::id()
        ))
    }
}
