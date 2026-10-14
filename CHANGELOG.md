# Changelog

## Unreleased (0.1)

First release under the name Manivelle. It continues `rustts` (deprecated, last
release 0.3.0); the history of `rustts` stays in that project's tags.

### Migration

- Coming from `rustts`: rename the crates, the `Vm*` types (`EngineError`,
  `EngineOptions`, `ContractValidation`, `UnknownFieldValidation`), the
  `#[manivelle(...)]` attributes, the `manivelle:env` module and the generated file
  names. See [Migrate From rustts](docs/guides/migrating-from-rustts.md).

### Added

- Embeddable TypeScript runtime: `oxc` transpiles, QuickJS runs scripts on the host's
  own thread through `Engine`, with no threads and no event loop.
- Typed host contracts (`HostFunction`, `HostCallback`, `HostContext`, `HostRequest`)
  registered through `TsSchema` and `JsEncode` / `JsDecode`, and the scripts'
  declarations and SDK generated from them (the `manivelle` command line tool).
- `#[derive(TsSchema)]` (feature `derive`) mirroring serde's wire format natively,
  with `#[manivelle(...)]` attributes for codec directions, the JSON opt-in, custom
  native codecs and schema overrides.
- Context groups (`load_script_in`, `load_project_in`), `ctx.hot` state across reloads,
  host-driven timers, `console`, async host functions resolved by the game loop,
  `Engine::interrupt_handle`, execution budgets, TypeScript source maps in errors,
  an optional on-disk transpile cache and `NativeBytes`.
- Web APIs in every script context (`ScriptBuiltins::web`): `structuredClone`,
  `TextEncoder` and `TextDecoder`, next to `atob`, `btoa` and `performance.now()`.
- A per-event index of the scripts that listen, so `emit` and `request` cost
  O(listeners) instead of O(scripts) (1000 scripts with 10 listeners: 13.3 to 3.5 us).
- Fault isolation: each script of an `emit`, `request`, `advance_timers` or `pump` gets
  an execution budget of its own, so a runaway script costs one timeout and the scripts
  after it still run. A script that runs out of its time or memory
  `EngineOptions::max_script_faults` times (3 by default) is quarantined, and
  `Engine::quarantined_scripts` lists it. See
  [Untrusted Mods](docs/guides/untrusted-mods.md).
- The `manivelle` tool: `manivelle init --from <program>` asks a program that opted in
  (`manivelle::export_sdk_if_requested`) for its scripting API and sets up a script
  project around the TypeScript declarations it gets; `manivelle sdk` writes them from
  a descriptors file.
