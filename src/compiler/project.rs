//! Multi-file project discovery: the static ESM graph reachable from an entry file,
//! rebuilt incrementally from the previous discovery of the same project.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use super::resolver::ModuleResolver;
use super::source_map::ModuleOrigin;
use super::stamp::{FileStamp, WatchedFiles};
use crate::error::EngineError;

/// Returns the static import requests of one module source. The path is the module's
/// display path: it names the module in diagnostics and gives its source type.
pub(crate) type ImportsOf<'a> =
    dyn FnMut(&str, &Path) -> Result<BTreeSet<String>, EngineError> + 'a;

/// One transpiled module, ready for the module store.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct CompiledModule {
    pub(crate) module_id: String,
    pub(crate) transpiled_js: String,
    pub(crate) origin: ModuleOrigin,
    pub(crate) resolved_requests: BTreeMap<String, String>,
}

pub(crate) struct DiscoveredProject {
    pub(crate) entry_module_id: String,
    pub(crate) modules: Vec<DiscoveredModule>,
    /// Module files plus everything resolution depended on.
    pub(crate) watched: WatchedFiles,
}

pub(crate) struct DiscoveredModule {
    pub(crate) module_id: String,
    /// The path relative to the project root, with `/` separators.
    pub(crate) display_path: String,
    pub(crate) source: String,
    pub(crate) resolved_requests: BTreeMap<String, String>,
}

/// A project load that failed, with the files it had looked at so far: module files,
/// the one that failed included, and what resolution depended on. A change to any of
/// them may fix the load, so they are watched like the files of a loaded version.
#[derive(Debug)]
pub(crate) struct ProjectFailure {
    pub(crate) error: EngineError,
    pub(crate) watched: WatchedFiles,
}

impl From<EngineError> for ProjectFailure {
    /// A failure before any project file was looked at.
    fn from(error: EngineError) -> Self {
        Self {
            error,
            watched: WatchedFiles::default(),
        }
    }
}

/// What the discovery of one project learned, reused by its next discovery.
///
/// Resolution only depends on the project structure (which files exist,
/// `tsconfig.json`, `package.json`), never on module contents. While the structure is
/// unchanged, the resolver keeps its file system cache and a module whose file is
/// unchanged is neither read, nor parsed, nor resolved again.
pub(crate) struct ProjectState {
    resolver: ModuleResolver,
    /// Directories from each module up to the project root, `tsconfig.json` and the
    /// `package.json` files resolution read.
    structure: WatchedFiles,
    /// Host module names imports were resolved against.
    external_modules: BTreeSet<String>,
    modules: HashMap<PathBuf, KnownModule>,
}

struct KnownModule {
    stamp: Option<FileStamp>,
    display_path: String,
    source: String,
    resolved_requests: BTreeMap<String, String>,
    dependencies: Vec<PathBuf>,
    package_jsons: Vec<PathBuf>,
}

/// Walks the static import graph from `entry_path`, reusing `previous` when it is the
/// state of the same project. `imports_of` returns the static import requests of one
/// module source; requests naming `external_modules` stay unresolved for the module
/// loader. Returns the graph and the state for the next discovery, or the error with
/// the files looked at before it.
pub(crate) fn discover_project(
    entry_path: &Path,
    external_modules: &BTreeSet<String>,
    previous: Option<ProjectState>,
    imports_of: &mut ImportsOf<'_>,
) -> Result<(DiscoveredProject, ProjectState), ProjectFailure> {
    let entry_path = normalize_entry_path(entry_path)?;
    let (resolver, structure, known, structure_unchanged) =
        reuse_state(previous, &entry_path, external_modules)?;
    let mut discovery = Discovery {
        resolver: &resolver,
        external_modules,
        imports_of,
        known,
        structure_unchanged,
        structure,
        modules: BTreeMap::new(),
    };
    // `paths` and `baseUrl` may come from any config of the `extends` chain.
    for tsconfig in resolver.tsconfig_files() {
        discovery.structure.watch(tsconfig);
    }
    let visited = discovery.visit(entry_path.clone());

    let Discovery {
        structure,
        modules: visited_modules,
        ..
    } = discovery;
    let mut watched = structure.clone();
    for (path, module) in &visited_modules {
        watched.watch_stamped(path, module.stamp);
    }
    if let Err(error) = visited {
        return Err(ProjectFailure { error, watched });
    }
    let mut modules = Vec::with_capacity(visited_modules.len());
    let mut known = HashMap::with_capacity(visited_modules.len());
    for (path, module) in visited_modules {
        modules.push(DiscoveredModule {
            module_id: module_id(&path),
            display_path: module.display_path.clone(),
            source: module.source.as_str().to_owned(),
            resolved_requests: module.resolved_requests.clone(),
        });
        known.insert(path, module);
    }

    Ok((
        DiscoveredProject {
            entry_module_id: module_id(&entry_path),
            modules,
            watched,
        },
        ProjectState {
            resolver,
            structure,
            external_modules: external_modules.clone(),
            modules: known,
        },
    ))
}

/// The resolver, the structure watched and the modules to start a discovery with: those of
/// `previous` when they still hold for this project, fresh ones otherwise. The flag tells
/// whether the previous resolutions still hold.
fn reuse_state(
    previous: Option<ProjectState>,
    entry_path: &Path,
    external_modules: &BTreeSet<String>,
) -> Result<
    (
        ModuleResolver,
        WatchedFiles,
        HashMap<PathBuf, KnownModule>,
        bool,
    ),
    EngineError,
> {
    match previous {
        Some(state)
            if state.external_modules == *external_modules && !state.structure.changed() =>
        {
            Ok((state.resolver, state.structure, state.modules, true))
        }
        Some(state) => Ok((
            ModuleResolver::for_entry(entry_path)?,
            WatchedFiles::default(),
            state.modules,
            false,
        )),
        None => Ok((
            ModuleResolver::for_entry(entry_path)?,
            WatchedFiles::default(),
            HashMap::new(),
            false,
        )),
    }
}

struct Discovery<'a, 'b> {
    resolver: &'a ModuleResolver,
    external_modules: &'a BTreeSet<String>,
    imports_of: &'a mut ImportsOf<'b>,
    /// Modules of the previous discovery.
    known: HashMap<PathBuf, KnownModule>,
    /// Whether the previous resolutions still hold.
    structure_unchanged: bool,
    structure: WatchedFiles,
    modules: BTreeMap<PathBuf, KnownModule>,
}

impl Discovery<'_, '_> {
    /// `path` is canonical: the entry is canonicalized once and resolution returns
    /// canonical paths, so it is also the module id.
    fn visit(&mut self, path: PathBuf) -> Result<(), EngineError> {
        if self.modules.contains_key(&path) {
            return Ok(());
        }

        // A file added in any directory between the module and the project root can
        // take precedence in resolution (`./src/value` -> `src/value.ts` over
        // `src/value/index.ts`), so every such directory is watched.
        for directory in path.ancestors().skip(1) {
            if !directory.starts_with(self.resolver.project_root()) {
                break;
            }
            self.structure.watch(directory);
        }

        // Stamped before reading: an edit racing this read shows up as a change.
        let stamp = FileStamp::of(&path);
        let module = match self.known.remove(&path) {
            Some(known) if stamp.is_some() && known.stamp == stamp && self.structure_unchanged => {
                Ok(known)
            }
            Some(known) if stamp.is_some() && known.stamp == stamp => {
                self.resolve_module(&path, stamp, known.source)
            }
            _ => fs::read_to_string(&path)
                .map_err(EngineError::from)
                .and_then(|source| self.resolve_module(&path, stamp, source)),
        };
        let module = match module {
            Ok(module) => module,
            Err(error) => {
                // Fixing this file may fix the project: it is watched as it was read.
                self.structure.watch_stamped(&path, stamp);
                return Err(error);
            }
        };
        for package_json in &module.package_jsons {
            self.structure.watch(package_json);
        }
        let dependencies = module.dependencies.clone();
        self.modules.insert(path, module);

        for dependency in dependencies {
            self.visit(dependency)?;
        }
        Ok(())
    }

    fn resolve_module(
        &mut self,
        path: &Path,
        stamp: Option<FileStamp>,
        source: String,
    ) -> Result<KnownModule, EngineError> {
        let display_path = display_path(self.resolver.project_root(), path);
        let requests = (self.imports_of)(&source, Path::new(&display_path))?;
        let mut resolved_requests = BTreeMap::new();
        let mut dependencies = BTreeSet::new();
        let mut package_jsons = Vec::new();

        for request in requests {
            if self.external_modules.contains(&request) {
                resolved_requests.insert(request.clone(), request);
                continue;
            }
            let resolved = self.resolver.resolve_request(path, &request)?;
            package_jsons.extend(resolved.package_json);
            resolved_requests.insert(request, module_id(&resolved.path));
            dependencies.insert(resolved.path);
        }

        Ok(KnownModule {
            stamp,
            display_path,
            source,
            resolved_requests,
            dependencies: dependencies.into_iter().collect(),
            package_jsons,
        })
    }
}

fn normalize_entry_path(entry_path: &Path) -> Result<PathBuf, EngineError> {
    if !entry_path.exists() {
        return Err(EngineError::Resolve {
            details: format!("entry path does not exist: {}", entry_path.display()),
        });
    }

    entry_path.canonicalize().map_err(EngineError::from)
}

fn module_id(path: &Path) -> String {
    path.to_string_lossy().into_owned()
}

/// How diagnostics and stack traces name a module: its path relative to the project
/// root with `/` separators, else its full path.
fn display_path(project_root: &Path, path: &Path) -> String {
    let Ok(relative) = path.strip_prefix(project_root) else {
        return module_id(path);
    };
    relative
        .iter()
        .map(|part| part.to_string_lossy())
        .collect::<Vec<_>>()
        .join("/")
}
