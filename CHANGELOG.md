# Changelog

## Unreleased

### Added

- `Engine::with_state(&mut state, |engine| ...)` lends a `&mut` the host owns to the host
  functions that run inside the closure, and `Caller::with_state::<T, _>(|state| ...)` reads
  it from a handler registered with `function_with_caller`, `async_function_with_caller` or
  `#[host_fn_with_ctx]`. Any call that can enter a script (`call`, `emit`, `request`,
  `advance_timers`, `pump`, `load_script`, `reload_changed`) runs inside the scope unchanged,
  so a host reaches its game world, editor or database without a global or a `Mutex`.
  The state is taken back when the closure returns or panics.
- `EngineError::NoState` (a handler asked for the state outside a scope; a script can
  catch it), `EngineError::StateType { expected }` (the scope lent another type),
  `EngineError::StateBusy` (a handler up the call stack holds the state) and
  `EngineError::StateScopeActive` (`with_state` inside `with_state`: scopes do not nest).

## 0.2.0

### Changed (breaking)

- `HostContractRegistry` is removed: its methods are inherent on `InMemoryHostContractRegistry`.
  The `register_*` names are gone (`register_function` → `function`, `register_function_with`
  → `function_with`, `register_function_with_caller` → `function_with_caller`,
  `register_async_function_with` → `async_function_with`,
  `register_async_function_with_caller` → `async_function_with_caller`, `register_callback`
  → `callback`, `register_request` → `request`, `register_context` → `context`).
- `InMemoryHostContractRegistry::descriptor(name)` is `get(name)`; `list()` is
  `descriptors()`; `typescript_declarations()` is `dts()` and `typescript_sdk()` is `sdk()`.
- `EngineError::ContractValidation::direction` is the new `Direction` enum
  (`Direction::Input` / `Direction::Output`, displayed as before) instead of `&str`.
- A host contract name registered twice fails with the new `EngineError::DuplicateContract
  { name }` instead of `EngineError::InvalidOptions`.
- `ScriptId` is a newtype (`pub struct ScriptId`) instead of `type ScriptId = String`. It
  shares its text on clone and derefs to `str`; it implements `Display`, `AsRef<str>`,
  `Borrow<str>`, `PartialEq<str>`, `From<&str>` and `From<String>`, and `as_str()` returns
  the text. Build one with `ScriptId::from(name)` and compare it with `== "name"`.
- `EngineError::ScriptNotFound::script_id` and `EngineError::FunctionNotFound::script_id`
  are `ScriptId` instead of `String`.
- `Emission<'a>` is `Emission`: it has no lifetime, and `failures` is
  `Vec<(ScriptId, EngineError)>` instead of `Vec<(&str, EngineError)>`, so it owns the ids.
- `ReloadReport::reloaded`, `ReloadReport::failed` and `ReloadReport::dispose_failed` hold
  `ScriptId` instead of `String`.
- `Engine::request` returns `Vec<(ScriptId, R)>` instead of `Vec<(&str, R)>`, and
  `Engine::request_deferred` returns `PendingCall<Vec<(ScriptId, R)>>` instead of
  `PendingCall<Vec<(String, R)>>`.
- `JsEncode::encode_js`, `JsDecode::decode_js` and `JsArgs::encode_args` return
  `Result<_, CodecError>` instead of `rquickjs::Result`. `CodecError` holds the path to the
  failing value as data (`path: Vec<PathSegment>`, where `PathSegment` is `Field`, `Index` or
  `Key`) and the cause. Its `Display` output is unchanged; `CodecError::at` adds a segment,
  and `From` converts it into `rquickjs::Error` at the QuickJS boundary.
- `Schema.name` is `Option<String>`: `None` is an anonymous shape rendered inline, replacing
  the `"unknown"` sentinel. `Schema::named` is removed; use `Schema::typed(name, ts_type)` or
  `Schema::anonymous(ts_type)`. `Schema::declared_name()` returns the alias name. A
  serialized `Schema` shows `null` where it showed `"unknown"`.
- `TsSchema::schema_name()` returns `Option<&'static str>`: `None`, the default, is an
  anonymous type. A hand-written impl returns `Some("Name")`. The derive output is
  unchanged in TypeScript.
- `manivelle::js::String` and `manivelle::js::Result` are removed: they shadowed the std
  names. `manivelle::js::Error` remains.
- The macro support items are reachable only through `#[doc(hidden)] manivelle::__private`
  (`__private::derive`, `__private::expect_object`, `__private::expect_array_len`,
  `__private::ObjectSchema`, `__private::schema_type_ref`, `__private::push_schema_dependency`,
  `__private::serde`). The nine `__xxx` re-exports and `manivelle::__serde` are removed.
  `codec::derive` is no longer a public module.

### Added

- `#[host_fn(name = "user.find")]` (feature `derive`): declares a host function
  from a plain `fn(input) -> Result<Output, EngineError>`. It generates the marker type
  (`FindUser`) with its `HostContract`, `HostFunctionSignature` and `HostFunction`
  impls, ready for `registry.function::<FindUser>()`. A function that cannot fail may
  return its output directly (or nothing) instead of a `Result`.
- `#[host_mod]` (feature `derive`), optional: on an inline module it prefixes the
  `#[host_fn]` names of its functions with the namespace (`#[host_mod(namespace = "math")]`,
  or the module's name when bare). Inside it, `#[host_fn]` takes the function's name when
  `name` is omitted, and `no_namespace` keeps `name` as the full contract name.
- `#[host_fn_with_ctx]` (function receiving the `Caller`) and `#[host_fn_async]` (function
  settling a `HostResolver` later), which generate the marker and a
  `register(&registry)` function.
- `ScriptExport` and `Engine::call_typed::<E>(script_id, args)`: the name, arguments and
  result of a script export declared once in Rust, so the call has no string to misspell.
- `HostContract::DOCS` and the `///` comments of a `#[host_fn]` function: they are written
  to `HostContractDescriptor::docs` and rendered as JSDoc in the generated declarations
  and SDK.
- `InMemoryHostContractRegistry::script_export::<E>()` (with `TsArgs` for the argument
  list): registers a `ScriptExport` so that the declarations and the SDK hold a
  `ScriptExports` type a script annotates its exports with, for `tsc` to check them.
- `manivelle run` shows the `console` output of the scripts (warnings and errors on
  stderr). While attached, the program's own default stderr output of `console` is
  silenced so a line shows once; a sink set with `set_console` still receives every line.
  The lines are flushed when the engine is dropped, so a program that ends right after a
  `console.log` still shows it (`std::process::exit` skips the drop).
- `manivelle check [DIR] [--packages DIR] [--from PROGRAM | --descriptors FILE] [--fix]`:
  validates packages without loading them (manifest, entry file, unique names, engine and
  dependency versions, cycles) and, with the program's contracts, **reads their TypeScript**:
  it flags a host function or event the code uses and the permissions do not list, a
  misspelled function or event (with the closest name), a permission that names no contract,
  a permission the code never uses, a `script_export` function the entry does not export, and
  computed access it cannot read, each with its file and line. `--fix` adds the permissions
  the code uses and the manifest lacks. The reading is static and best effort.
- The library part of it: `resolve_packages`, `analyze_project` (what a project uses, found
  with `oxc`'s parser and scope analysis, so a local named like a host namespace is not
  mistaken for it), `check_package`, `ContractNames` and `add_permissions`, usable from a
  `build.rs` or a test.
- `manivelle init` also writes a `manivelle.kdl` named after the project, with the program as
  its `target` and its host functions and events listed, commented out, under `permissions`.
- `#[host_event(name = "user.found")]` (feature `derive`): declares an event from its payload
  type, with `reply = Type` for a request; `#[host_mod]` prefixes event names too. The `///`
  comments of an event become JSDoc in the declarations and the SDK.
- `#[script_export(name = "run", args = (Tick,), output = Forecast)]` (feature `derive`):
  writes the `ScriptExport` impl of a struct.
- Packages: a `manivelle.kdl` manifest, in KDL 2 (`PackageManifest`), with the package's
  name, version, author, `engine` requirement, `depends`, `permissions`, `entry` and
  `target`, and `Engine::load_packages(&[dirs])`, which checks the whole set (unique names,
  engine and dependency versions, cycles) before loading anything, then loads each package
  as its own project, dependencies first. A package that fails does not stop the others,
  only its dependents. A script loaded with `load_script` or `load_project` needs no
  package and is not restricted.
- Package `permissions` are enforced: a package's script can call only the host functions,
  and listen only to the events, whose contract names it lists (`world.modify`, `world.*`,
  or `*`); a package that lists none reaches no host contract. A refused function is still
  there, and calling it throws `permission denied: package ...` (`EngineError::PermissionDenied`);
  `ctx.on` of a refused event throws the same.
- `Engine::setup_protocol` and `Engine::poll_protocol`: one call at startup that does
  what the `manivelle` tool asked (write the SDK for `init`, connect, load the packages),
  and one per frame that runs the tool's requests (export the SDK, list contracts and
  packages, reload) on the engine's own thread. The connection starts one thread that only
  reads and queues requests; it never touches the engine. Authenticated by a random token,
  bound to `127.0.0.1`, defined in `manivelle::protocol`. `export_sdk_if_requested` is
  kept and `setup_protocol` calls it.
- `manivelle run [DIR] [--no-packages] [--packages DIR] [--watch] [--target PATH]`: starts
  the target named by the manifest with its packages, or vanilla with `--no-packages`, and
  prints what loaded; `--watch` reloads changed package files while it runs.

### Fixed

- On Windows, a program that ended right after `Engine::setup_protocol` reset its connection
  to `manivelle run`, which lost the greeting and reported that the target exited before
  connecting. Dropping the engine now closes the connection in order.

## 0.1.2

### Added

- A per-script memory quota, `EngineOptions::script_memory_limit_bytes` (off by default,
  and then free): the engine charges each script for what it allocates while it runs
  and credits it when it frees it, so a script over its quota fails alone with an error
  that names it, counted as a fault, while the others of the engine keep their share.
  `Engine::script_memory_bytes` reads what a script holds. It is accounting inside one
  pool, not isolation: see [Untrusted Mods](docs/guides/untrusted-mods.md).

## 0.1.1

### Added

- `Engine::emit_to` and `Engine::request_to` deliver an event to the handlers of one
  script only.
- `Engine::emit_report` returns an `Emission`: how many scripts an event reached and the
  error of each script that failed, instead of only the first error of the emission.

## 0.1.0

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
- The `manivelle` tool (feature `cli`, `cargo install manivelle --features cli`):
  `manivelle init --from <program>` asks a program that opted in
  (`manivelle::export_sdk_if_requested`) for its scripting API and sets up a script
  project around the TypeScript declarations it gets; `manivelle sdk` writes them from
  a descriptors file.
- Hardening for untrusted scripts: values crossing into Rust as JSON nest at most 128
  levels (a cyclic value is an error, not a stack overflow), `TextEncoder` output counts
  against `memory_limit_bytes`, Promise jobs are charged to the script that queued them,
  and `EngineOptions` refuses a stack or memory limit of 0 (`EngineError::InvalidOptions`).
