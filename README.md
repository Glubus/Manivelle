# Manivelle

**Run TypeScript inside your Rust application, with the types generated from your Rust code.**

Manivelle embeds a TypeScript runtime (QuickJS, transpiled by `oxc`) in your program. You
expose Rust functions and events to scripts; Manivelle generates the TypeScript
declarations for them, so whoever writes the scripts (you, a designer, a modder) gets
autocompletion and type errors against your real API. There is no Node or Deno to ship,
no event loop and no thread of its own: scripts run on your thread when you call them.

Manivelle is licensed under MIT. It is the successor of `rustts`, which is
deprecated: see [Migrate From rustts](docs/guides/migrating-from-rustts.md) and
[the changelog](CHANGELOG.md).

## What It Is For

- game UI and gameplay rules, and modding APIs for a game
- plugin systems inside Rust applications
- editor or tool automation
- simulation rules written in TypeScript
- any Rust host that wants a typed scripting layer without embedding Node or Deno

It is a scripting layer, not a server runtime.

## A First Look

The Rust side declares what scripts may call (`cargo add manivelle --features derive`):

```rust
use manivelle::{Engine, EngineError, EngineOptions, TsSchema, host_fn};
use serde::{Deserialize, Serialize};

#[derive(Deserialize, TsSchema)]
struct AddInput { left: i32, right: i32 }

#[derive(Serialize, TsSchema)]
struct AddOutput { value: i32 }

/// Adds two numbers.
#[host_fn(name = "math.add")]
fn add(input: AddInput) -> Result<AddOutput, EngineError> {
    Ok(AddOutput { value: input.left + input.right })
}

fn main() -> Result<(), EngineError> {
    let mut engine = Engine::new(&EngineOptions::default())?;
    engine.registry().function::<Add>()?;

    engine.load_script("rules", "export function run() { return math.add({ left: 20, right: 22 }); }")?;
    let result: serde_json::Value = engine.call("rules", "run", ())?;
    println!("{result}"); // {"value":42}
    Ok(())
}
```

The script calls `math.add(...)` as if it were TypeScript, and the declarations Manivelle
generates for it (`declare namespace math { export function add(input: AddInput): AddOutput; }`,
with the `///` comment as JSDoc) come from the same Rust function.

## Use It As A Library: No Tool Needed

**Manivelle is a Rust crate, and that is all you need.** Everything above and below works
without installing anything else, and a game or application you ship never depends on the
`manivelle` command:

- load scripts and projects: `load_script`, `load_project` (multi-file, `tsconfig` paths,
  `node_modules`), and `reload_changed` for hot reload;
- call exports and emit events: `call`, `call_typed`, `emit`, `request`, timers, async host
  functions;
- generate the TypeScript declarations and SDK for script authors yourself, from your own
  code: `registry().write_sdk_files(...)`;
- load packages (scripts with a manifest, dependencies and permissions) from a directory:
  `load_packages(&[dir])`, and the permissions are enforced by the library.

## The Optional `manivelle` Tool

`cargo install manivelle --features cli` adds a command for the people who *write* scripts
for a program that embeds Manivelle. It is development tooling; nothing in your program
needs it at runtime.

| Command | What it does |
| --- | --- |
| `manivelle init --from ./game` | Asks the program for its API and sets up a script project: the declarations, a `tsconfig.json`, a first script and a `manivelle.kdl`. |
| `manivelle run` | Starts the target program named by the manifest with the package loaded, and shows the scripts' `console` output. `--no-packages` starts it vanilla, `--watch` reloads on change. |
| `manivelle check --from ./game` | Validates the packages, reads their TypeScript against the program's contracts (unknown function, missing or unused permission) and flags a typo with the closest name. `--fix` adds the missing permissions. |

For these commands to talk to your program, it calls `engine.setup_protocol()` once at
startup and `engine.poll_protocol()` once per frame. Started by hand, a program that calls
them behaves exactly as if it did not: they do nothing without the tool.
See [Packages And The manivelle Tool](docs/guides/packages-and-the-manivelle-tool.md).

## Packages And Permissions

A **script** is what you load yourself, and it is trusted. A **package** is a script
project with a `manivelle.kdl` manifest, for what others write or what you distribute:

```kdl
package "better-combat" {
    version "1.2.0"
    depends { "core" version "^1.0" }
    permissions {
        world.modify
        entity.*
    }
    entry "src/index.ts"
    target "../game"
}
```

The library loads a set of packages in dependency order, checks versions and cycles, and
**enforces the permissions**: a package can only call the host functions and listen to the
events it lists. A package is optional: a simple script needs none.

## Declaring The Rust Side

With the `derive` feature, attributes write the boilerplate:

- `#[host_fn(name = "user.find")]` on a function declares a host function (zero or one
  parameter, `Result<T, EngineError>` or a plain `T`); `#[host_fn_with_ctx]` also receives the
  calling script, `#[host_fn_async]` settles a `Promise` later;
- `#[host_event(name = "user.found")]` on the payload struct declares an event;
  `reply = Type` makes it a request;
- `#[host_mod(namespace = "math")]` on an inline module prefixes the names of its functions
  and events;
- `#[script_export(name = "run", args = (Tick,), output = Forecast)]` declares a function the
  script exports, to call with `engine.call_typed::<Run>(id, (tick,))`; register it with
  `registry().script_export::<Run>()` and the generated declarations hold a `ScriptExports`
  type that a script annotates its exports with for `tsc` to check them.

Without the macros, the same contracts are the `HostContract`, `HostFunctionSignature`,
`HostFunction` and `HostCallback` traits; see
[Register Host Functions And Callbacks](docs/guides/register-host-functions.md).

Scripts call host functions and handle host events:

```ts
let lastFound = "none";

ctx.on("user.found", event => {
  lastFound = event.displayName;
});

export function run() {
  return user.find({ userId: 7 });
}
```

## Documentation

The docs are an mdBook source tree in [`docs/`](docs/SUMMARY.md).

Useful starting points:

- [Getting Started](docs/getting-started.md)
- [Run Scripts With Engine](docs/guides/engine.md)
- [Register Host Functions And Callbacks](docs/guides/register-host-functions.md)
- [Generate TypeScript SDK Files](docs/guides/generate-sdk-files.md)
- [Packages And The manivelle Tool](docs/guides/packages-and-the-manivelle-tool.md)
- [Rust And TypeScript Types](docs/guides/type-mapping.md)
- [Load Scripts And Projects](docs/guides/load-scripts-and-projects.md)
- [Use Native Bytes](docs/guides/native-bytes.md)
- [Runtime Guarantees](docs/guides/runtime-guarantees.md)
- [Untrusted Mods](docs/guides/untrusted-mods.md)
- [Migrate From rustts](docs/guides/migrating-from-rustts.md)

[`examples/getting_started.rs`](examples/getting_started.rs) is the smallest complete
program (one Rust function called from a script), and
[`examples/game_loop.rs`](examples/game_loop.rs) runs a frame loop over a context
group with timers, events, requests, an async host function and a hot reload that
keeps state: `cargo run --example game_loop --features derive`.

Build the book with:

```text
mdbook build
```

## Status

Manivelle provides:

- `Engine`: QuickJS (through `rquickjs`) on your own thread, with direct native
  calls in both directions
- TypeScript transpilation through `oxc`, with an optional disk cache; errors and
  stacks point at TypeScript file, line and column
- inline scripts and static ESM project graphs
- context groups: scripts that trust each other share one QuickJS context
  (`load_script_in`, `load_project_in`), cheaper to load and to deliver events to
- hot reload that keeps script state through `ctx.hot` (`save`, `data`, `dispose`)
- typed host functions, as static functions or closures that can see the calling
  script, and callbacks, including requests whose handlers reply (`Engine::request`),
  declared by hand or with `#[host_fn]`, `#[host_event]` and their siblings
- async host functions answered by the host from any thread (`async_function_with`,
  `HostResolver`, `Engine::pump`), and deferred script results across frames
  (`call_deferred`, `request_deferred`, `PendingCall`)
- `ctx.on` / `ctx.off`, `console` routed to the host, and timers on a clock the host
  advances (`advance_timers`)
- native Rust ↔ JavaScript value conversion with `serde_json` semantics
- generated TypeScript declarations and SDK helpers, with the `///` comments of the Rust
  code as JSDoc, and typed script exports (`ScriptExport`, `ScriptExports`)
- packages: a `manivelle.kdl` manifest (KDL 2), dependencies checked as a set, and enforced
  permissions
- the optional `manivelle` tool (`init`, `run`, `check`) and the `setup_protocol` /
  `poll_protocol` link a program opts in to (`cargo install manivelle --features cli`)
- optional contract validation
- native `Uint8Array` in both directions through `NativeBytes`
- web APIs QuickJS lacks, loaded only when a script uses them: `structuredClone`,
  `TextEncoder` and `TextDecoder` (next to `atob`, `btoa` and `performance.now()`)
- execution budget per script for each load, call, emit, request and timer advance
  (a script that keeps running out of it is quarantined), and an
  `InterruptHandle` to stop running JavaScript from another thread
- garbage collection control (`run_gc`, `set_gc_threshold`) and per-context
  built-in selection (`EngineOptions::builtins`, `ScriptBuiltins`)
- the `disable-assertions` feature: QuickJS without its internal assertions, for
  the builds you ship

## Performance

In one `cargo bench --bench vs_lua` run on an otherwise idle Windows development
machine, a Rust → TypeScript call with two numbers took 83 ns against 52 ns for mlua,
a round trip with a 20-field object 3.9 µs against 4.4 µs, and an event to one
handler 143 ns against 264 ns. Pure
compute is QuickJS speed, about 3× slower than Lua. Context groups and the
`disable-assertions` feature (about 23 % faster on call-heavy scripts) are described in
[the Engine guide](docs/guides/engine.md#performance), with the full comparison.

`poll_protocol`, called once per frame, costs about 0.7 ns when no tool is attached and
about 0.9 ns when one is attached and idle (a release build on Linux).

## Toolchain

The repository pins its Rust toolchain in [`rust-toolchain.toml`](rust-toolchain.toml).

Local checks:

```text
cargo fmt --all --check
cargo clippy --workspace --features derive,uuid,chrono,glam --all-targets -- -D warnings
cargo test --workspace --features derive,uuid,chrono,glam
cargo test --workspace --no-default-features
cargo check --workspace --all-features --all-targets
cargo doc --workspace --no-deps --features derive,uuid,chrono,glam   # RUSTDOCFLAGS="-D warnings"
mdbook build
```

Tests run without `disable-assertions`, so QuickJS assertions stay on; the
`--all-features` check only proves that feature builds.

Install the pinned SDK type checker with `npm ci` before running the tests.
CI requires it; locally, set `MANIVELLE_REQUIRE_TSC=1` to enforce the same rule.

Execution budget, reload and cache guarantees are documented in
[Runtime Guarantees](docs/guides/runtime-guarantees.md).
