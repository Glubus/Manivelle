# Generate TypeScript SDK Files

`Manivelle` generates TypeScript files from the Rust-side registry. This is how a
host application exposes a typed scripting API to script authors: the Rust
contracts are the single source of truth, and the declarations follow them.

## Register Contracts First

The registry is the source of truth. Register every host function and callback
before generating files.

```rust
let engine = Engine::new(&options)?;

engine
    .registry()
    .function::<FindUser>()?
    .callback::<UserFound>()?;
```

Every registered callback appears in the declarations and the SDK.

## Write Files

```rust
use manivelle::SdkFileNames;

let written = engine.registry().write_sdk_files_with_names(
    "generated",
    &SdkFileNames {
        types: "my_sdk.d.ts".into(),
        sdk: "my_sdk.ts".into(),
    },
)?;

assert!(written.types_path.ends_with("my_sdk.d.ts"));
assert!(written.sdk_path.ends_with("my_sdk.ts"));
```

This creates:

```text
generated/
  my_sdk.d.ts
  my_sdk.ts
```

`write_sdk_files(...)` also exists for quick experiments (it writes
`manivelle.d.ts` and `manivelle.sdk.ts`), but real applications should usually call
`write_sdk_files_with_names(...)` and choose names that match their package.
`registry().dts()` and `registry().sdk()` return the same contents as strings.

## Import Modules

File names do not define an import name. A contract becomes importable when it
sets both `IMPORT_MODULE` and `EXPORT_PATH`:

```rust
impl HostContract for FindUser {
    const NAME: &'static str = "user.find";
    const IMPORT_MODULE: &'static str = "my_sdk";
    const EXPORT_PATH: &'static [&'static str] = &["user", "find"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}
```

Then a script can use:

```ts
import { user } from "my_sdk";

const result = user.find({ userId: 7, includeRoles: true });
```

`Engine` provides `my_sdk` as a virtual module built from the registry: a host
function is exported at its `EXPORT_PATH`, and a callback is exported as a function
that registers a handler (`user.found(handler)` for
`EXPORT_PATH = ["user", "found"]`). Use the same `IMPORT_MODULE` for every contract
that should be exported from the same module. Contexts are not exported: they
are declarative metadata.

`#[host_mod(namespace = "user", module = "my_sdk")]` sets both for every
`#[host_fn]` and `#[host_event]` of an inline
module, with the contract name as the export path (`user.find` is `["user", "find"]`).

## What The Declaration File Contains

The declaration file contains the public TypeScript API:

- host function input and output types
- callback payload types
- the `ctx` global: `ctx.hot` always (see
  [Keep State Across Reloads](engine.md#keep-state-across-reloads)), and
  `ctx.on(...)` once a callback is registered
- generated namespaces such as `user.find(...)`: the globals of a script outside any
  context group
- a `declare module "<IMPORT_MODULE>"` block for every import module, for the scripts
  that import (a grouped script has no globals)
- event maps for typed callback subscriptions

The `declare module` blocks mirror the virtual module `Engine` builds, so
`import { user } from "my_sdk"` type-checks from the declaration file alone, with no
SDK file and no `paths` mapping. For `#[host_mod(namespace = "math", module = "app")]`
and a documented `add` function:

```ts
declare namespace math {
  /** Adds two numbers. */
  export function add(input: AddInput): AddOutput;
}

declare module "app" {
  export const math: {
    /** Adds two numbers. */
    add(input: AddInput): AddOutput;
  };
}
```

An `EXPORT_PATH` of one segment is an exported function (`export function find(...)`),
a longer path an exported object holding its functions, and a callback is the function
that registers a handler (`onFound(handler: HostEventHandler<"user.found">): void`).
Modules and their members come out in name order. A contract with an `IMPORT_MODULE`
appears in both forms, because a script of a context of its own has the global as well
as the import; a contract without one is a global only, and a script in a group cannot
reach it.

Use it from a `tsconfig.json` with `include` or `typeRoots`, depending on how
your application packages scripts.

## What The SDK Source File Contains

The SDK file is an ordinary TypeScript module: typed wrappers around the host
functions (through the `__host.callValue` bridge that `Engine` installs in every
script), event helpers, and model helpers for object schemas. A project imports it
like any local file, for example `import { user } from "./generated/my_sdk";`; an
inline script can be loaded with the SDK source in front of it
(`format!("{sdk}\n{script}")`).

For a Rust contract named `user.find`, the SDK exposes:

```ts
const result = user.find({ userId: 7, includeRoles: true });
```

For a callback named `user.found`, it exposes:

```ts
ctx.on("user.found", event => {
  event.displayName.toUpperCase();
});

user.onFound(event => {
  event.roles.length;
});
```

## Type The Exports A Script Must Provide

`Engine::call_typed::<E>` checks, in Rust, the name, arguments and result of an export.
To have TypeScript check the script too, register the same declaration:

```rust
struct Run;
impl ScriptExport for Run {
    const NAME: &'static str = "run";
    type Args = (Tick,);   // each argument implements TsSchema
    type Output = Forecast;
}

engine.registry().script_export::<Run>()?;
```

The generated declarations then hold a `ScriptExports` type (`export type` in the SDK
module), and a script annotates its export with it:

```ts
export const run: ScriptExports["run"] = (arg0) => forecast(arg0);
```

`tsc` then rejects an export with the wrong parameters or result; a script may also be
`async`. An export is only described: scripts cannot call it, and it is not a host contract.
`#[script_export(name = "run", args = (Tick,), output = Forecast)]` on a unit struct writes
the `ScriptExport` impl for you.

## Give Mod Authors The Declarations: `manivelle init`

Mod authors need your declarations on their machine, in a project that type-checks.
You do not have to ship a tool for that: your program can hand its scripting API to the
`manivelle` command line tool. Install it with `cargo install manivelle --features cli`.

Opt in by calling `Engine::setup_protocol` once, right after registering the contracts
and before the program does anything else at startup:

```rust
let mut engine = Engine::new(&EngineOptions::default())?;
engine.registry().function::<FindUser>()?.callback::<UserFound>()?;

if engine.setup_protocol()?.exported {
    return Ok(()); // the tool asked for the SDK: it is written, nothing else to do
}
// ... open the window, load the assets, run the game
```

Without a request from the tool it does nothing. The program is never inspected from
outside: only a program that calls it can be exported. The same call lets
[`manivelle run`](packages-and-the-manivelle-tool.md) load packages into the program. A program
that only wants the SDK export can call `export_sdk_if_requested` alone.

A mod author then runs, with your executable (a game, an editor, a server):

```text
manivelle init my-mod --from "path/to/game.exe"
```

The tool starts the program with the environment variable `MANIVELLE_EXPORT_SDK` set to
a temporary directory, waits for it to exit (30 seconds by default, `--timeout` changes
it), and creates:

```text
my-mod/
  manivelle/manivelle.d.ts      # your contracts, always refreshed
  manivelle/manivelle.sdk.ts    # the typed SDK module
  tsconfig.json                 # created only when missing
  src/main.ts                   # created only when missing
```

Run the same command again after a game update to refresh `manivelle/`; the author's
`tsconfig.json` and scripts are never overwritten. `--types` and `--sdk` rename the
generated files, and arguments after `--` are passed to the program (a game that needs
`--headless`, say). Running a program means trusting it: only use `--from` on programs you
would run anyway.

`manivelle sdk descriptors.json out-dir` writes the same files from a descriptors file,
for build scripts that never start the program: the descriptors are
`engine.registry().descriptors()` serialized as JSON.

## Describe The API As JSON

`engine.registry().api_schema()` returns the same contracts as one `serde_json::Value`,
laid out like an OpenAPI document, for documentation generators, API discovery and tools
that do not read TypeScript:

```json
{
  "manivelle": 1,
  "jsonSchemaDialect": "https://json-schema.org/draft/2020-12/schema",
  "functions": {
    "math.add": {
      "description": "Adds two numbers.",
      "module": "app",
      "path": ["math", "add"],
      "input": { "$ref": "#/schemas/AddInput" },
      "output": { "$ref": "#/schemas/AddOutput" }
    }
  },
  "events": {},
  "exports": {},
  "contexts": {},
  "schemas": {
    "AddInput": {
      "type": "object",
      "properties": { "left": { "type": "number" }, "right": { "type": "number" } },
      "required": ["left", "right"]
    },
    "AddOutput": {
      "type": "object",
      "properties": { "value": { "type": "number" } },
      "required": ["value"]
    }
  }
}
```

- `functions` are host functions, `events` the callbacks and requests (`payload`, and
  `reply` for a request), `exports` the functions scripts export (`args`, `output`).
- `description` is the `///` comment of the Rust item. `module` and `path` are present
  when the contract is importable (`import { math } from "app"`).
- A function that takes or returns nothing has no `input` or `output`; one that returns
  a `Promise` has `"async": true`.
- Types are JSON Schema 2020-12. A named Rust type is listed once under `schemas` and
  referenced with `$ref`. An optional field is left out of `required`; a `Uint8Array` is
  the array of its bytes with `"format": "uint8array"`; an enum with fields is a `oneOf`
  of objects told apart by its tag.
- `manivelle` is the version of this layout, raised when a reader of the previous one
  would misread the new one.

Unlike `descriptors()`, which serializes the registry's own Rust types so that
`manivelle sdk` can read them back, this document is meant to be read by other tools.
