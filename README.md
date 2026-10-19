# Manivelle

Embeddable TypeScript scripting for Rust applications and games.

Manivelle is licensed under MIT. It is the successor of `rustts`, which is
deprecated: see [Migrate From rustts](docs/guides/migrating-from-rustts.md) and
[the changelog](CHANGELOG.md).

`Manivelle` lets a Rust host run TypeScript scripts on its own thread through
`Engine`, expose typed Rust host functions and callbacks to them, and generate
the scripts' TypeScript declarations and SDK from those Rust contracts. It spawns
no threads and has no event loop: scripts run when the host calls them. It is a
scripting layer, not a server runtime.

## What It Is For

- game UI and gameplay rules
- modding APIs
- plugin systems inside Rust applications
- editor or tool automation
- simulation rules written in TypeScript
- any Rust host that wants a typed scripting layer without embedding Node or Deno

## Core Flow

1. Declare Rust host contracts: `HostContract` names a function or event,
   `HostFunctionSignature` gives a function its `Input` and `Output`, and
   `HostFunction` implements it as a static function (or register a closure with
   `function_with`). Events implement `HostCallback`.
2. Derive `TsSchema` for input/output payloads.
3. Register contracts in the engine's registry.
4. Generate TypeScript declaration and SDK files for your package.
5. Load TypeScript scripts, call their exports and emit events to them.

```rust
struct FindUser;

impl HostContract for FindUser {
    const NAME: &'static str = "user.find";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindUser {
    type Input = FindUserInput;   // #[derive(TsSchema)]
    type Output = FindUserOutput; // #[derive(TsSchema)]
}

impl HostFunction for FindUser {
    fn call(input: FindUserInput) -> Result<FindUserOutput, EngineError> {
        Ok(FindUserOutput { display_name: format!("user {}", input.user_id) })
    }
}

let mut engine = Engine::new(&EngineOptions::default())?;

engine
    .registry()
    .function::<FindUser>()?
    .callback::<UserFound>()?;

engine.registry().write_sdk_files_with_names("generated", &SdkFileNames {
    types: "my_sdk.d.ts".into(),
    sdk: "my_sdk.ts".into(),
})?;
engine.load_script("weather-rules", include_str!("weather_rules.ts"))?;

let output: serde_json::Value = engine.call("weather-rules", "run", ())?;
engine.emit("user.found", &UserFoundPayload { user_id: 7, display_name: "Ada".into() })?;
```

With the `derive` feature, `#[host_fn]` writes the marker and its three impls from a
plain function, and the optional `#[host_mod]` prefixes the names of a module's functions:

```rust
#[host_fn(name = "user.find")]
fn find_user(input: FindUserInput) -> Result<FindUserOutput, EngineError> {
    Ok(FindUserOutput { display_name: format!("user {}", input.user_id) })
}

engine.registry().function::<FindUser>()?;
```

See [Register Host Functions And Callbacks](docs/guides/register-host-functions.md).

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
  script, and callbacks, including requests whose handlers reply (`Engine::request`)
- async host functions answered by the host from any thread (`async_function_with`,
  `HostResolver`, `Engine::pump`), and deferred script results across frames
  (`call_deferred`, `request_deferred`, `PendingCall`)
- `ctx.on` / `ctx.off`, `console` routed to the host, and timers on a clock the host
  advances (`advance_timers`)
- native Rust ↔ JavaScript value conversion with `serde_json` semantics
- generated TypeScript declarations and SDK helpers, and a `manivelle` tool that
  sets up a mod project from a program that opted in
  (`manivelle init --from game.exe`, `cargo install manivelle --features cli`)
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
