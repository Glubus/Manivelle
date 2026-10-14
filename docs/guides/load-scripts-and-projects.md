# Load Scripts And Projects

`Engine` loads TypeScript code and keeps it loaded until it is replaced or
unloaded. After a script is loaded, Rust calls its exported functions by:

- `script_id`: the host application's stable name for that loaded script
- export name: the TypeScript function exported by the script

`script_id` is not a file path and not a package name. It is an id chosen by
your application. Good examples:

- `weather-rules`
- `mod:zoro_moveset`
- `hud:inventory`

Bad examples:

- `plugin`, unless your app really has only one generic plugin
- a random UUID if you want reloads and diagnostics to stay readable

## Single Script Source

Use `load_script` when you already have one TypeScript source string.

```rust
use manivelle::{Engine, VmOptions};
use serde_json::{Value, json};

let mut engine = Engine::new(&VmOptions::default())?;
let script_id = "weather-rules";

engine.load_script(
    script_id,
    r#"
export function greet(input: { name: string }): string {
  return `hello ${input.name}`;
}
"#,
)?;

let output: Value = engine.call(script_id, "greet", vec![json!({ "name": "Ada" })])?;
assert_eq!(output, json!("hello Ada"));
```

What the arguments mean:

- `script_id`: `"weather-rules"` is the id of this loaded script.
- `"greet"`: exported TypeScript function name.
- `vec![...]`: arguments passed to the exported function, one per item. A tuple
  such as `(&input, 3)` passes values of different Rust types.

An inline script may import the registered host modules, not other files: use a
project for those.

## Project Entrypoint

Use `load_project` when the script lives on disk and imports other local files.

```rust
engine.load_project("zoro-moveset", "mods/zoro_moveset/main.ts")?;
```

Example project:

```text
mods/zoro_moveset/
  main.ts
  math.ts
  tsconfig.json
```

```ts
// main.ts
import { bonus } from "./math";

export function run() {
  return bonus(40) + 2;
}
```

Then Rust calls:

```rust
let output: f64 = engine.call("zoro-moveset", "run", ())?;
```

## `load_script` vs `load_project`

Use `load_script` when:

- your host already has the TypeScript source in memory
- the script is a single file
- you do not need local imports

Use `load_project` when:

- the entry file is on disk
- the script imports local modules
- you want `tsconfig.json` `baseUrl` / `paths`
- you want package imports from project-local `node_modules`

## Supported Project Imports

The project graph is resolved from the entry file, following:

- static relative ESM imports and re-exports
- named, default, and namespace imports
- extensionless local resolution
- `index.*` resolution
- `tsconfig.json` `baseUrl` and `paths`
- package imports from project-local `node_modules`
- type-only imports and re-exports, ignored at runtime

Packages resolve as ES modules: `exports` conditions `import`, `module` and
`default` (never `require` or `node`), and for a package without `exports`, its
`module` field before `main`.

An import naming a registered host module (a contract's `IMPORT_MODULE`) always
resolves to that module, never to a file or package of the same name. A script
never reaches another script's modules: only the host modules and its own graph
resolve, in a context group too.

The project root is the directory of the nearest `tsconfig.json` above the entry
file, or the entry file's directory when there is none. An import that resolves
outside the project root, symlinks included, is rejected.

Dynamic `import(...)` is rejected with `VmError::Resolve` (`dynamic import() is not
supported; use a static import`), in projects and inline scripts alike: the whole
graph must be known at load. A missing entry file or an unresolvable import fails
the load with `VmError::Resolve` too.

## Reloading

Loading the same `script_id` again replaces the loaded script:

```rust
engine.load_project("zoro-moveset", "mods/zoro_moveset/main.ts")?;
```

The new version is transpiled, resolved and initialized before it replaces the
old one. If any step fails, the load returns the error and the previous version
stays loaded, with its state. A successful reload starts from fresh script state,
except for what the script hands over through `ctx.hot` (see
[Keep State Across Reloads](engine.md#keep-state-across-reloads)), and keeps the
script's place in event delivery order.

To reload projects when their files change, call `engine.reload_changed()` from
your loop, for example once per second during development: it reloads the projects
whose files changed and reports failures, without starting a thread. It watches the
modules and their directories, the `tsconfig.json` and the configs it `extends`
through a relative or absolute path (not configs from packages), the `package.json`
files resolution read, and, after a failed reload, the files that reload reached.
See [Hot Reload](engine.md#hot-reload).

## Unloading

```rust
engine.unload_script("zoro-moveset")?;
```

Unloading runs the script's `ctx.hot.dispose` callbacks, then removes the script and
releases its modules. Calling or unloading it afterwards fails with
`VmError::ScriptNotFound`.

## Transpilation Cache

By default (`VmOptions::cache_dir: None`), transpiled modules are only remembered in
memory by the engine. With a cache directory, they are also stored on disk and
reused by later loads, including in later runs of the application.

The cache works per module: changing one file of a project transpiles that file
only. Import resolution is never cached on disk. Cache I/O never fails a
load: an unreadable artifact is a miss, an unwritable one is skipped. The directory
is never purged; delete it to reclaim space. See
[Run Scripts With `Engine`](engine.md#transpilation-cache).
