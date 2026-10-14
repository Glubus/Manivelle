# Runtime Guarantees And Limits

## TypeScript and contracts

OXC transpiles TypeScript and checks syntax; loading a script does **not** perform
TypeScript type checking. Run `tsc --noEmit` on author projects. Rust contract
schemas generate SDK declarations; runtime schema validation is independently
controlled by `contract_validation` and `unknown_field_validation`.

Repository SDK tests use the pinned TypeScript package installed by `npm ci`.
CI (or `MANIVELLE_REQUIRE_TSC=1`) fails if that compiler is missing. Local Rust-only
development can skip these checks when the package is absent, with a diagnostic.

## Threads and isolation

`Engine` runs every load, call, emit, request and `advance_timers` on the thread
that owns it, one at a time. Manivelle starts no threads and runs no background work:
between two calls, no script code runs, and timers only fire in `advance_timers`.

Each script owns a separate QuickJS context and ESM module graph, so scripts do not
share globals, unless they are loaded into one context group: scripts loaded with
`Engine::load_script_in` or `load_project_in` under the same group name share one
context, its built-ins and its global object. A group is a trust boundary: put only
scripts that trust each other in one (see
[Context Groups](engine.md#context-groups)). Scripts of the same `Engine` share its
QuickJS runtime: memory limit, stack limit and garbage collector. This is not
process isolation or a per-script memory quota. Host functions are capabilities
granted to every script of that engine.

A script's imports resolve only to the registered host modules and to the modules of
its own graph. One script cannot import another script's modules, even by naming
their internal module id, and that holds within a context group too.

## Execution budget

`EngineOptions::execution_timeout` defaults to 5 seconds. It limits each load,
call, emit, request, deferred start, `pump`, and `advance_timers`, starting
when that operation starts, after transpilation and module resolution: host
function time and Promise jobs count. An `emit`, `request`, `pump` or
`advance_timers` gives **each script it reaches a budget of its own**, so a script
that never returns costs one timeout and the scripts after it still run. Suspended
time between host operations does not consume a budget.

QuickJS's interrupt handler stops JavaScript still running when the budget
expires; the operation fails with `EngineError::Execution` and the engine stays
usable. Limits are cooperative, not real-time scheduling guarantees. A Rust host
function cannot be preempted: it must return, have its own I/O timeouts, and avoid
unbounded blocking. For hostile native extensions, use a separate process.
JavaScript state mutations and host side effects made before the interruption
remain.

`Engine::interrupt_handle` returns an `InterruptHandle` (`Send + Clone`) that stops
the running load, call, emit, request or timer advance from another thread, at the
same QuickJS interrupt checks: the operation fails with `EngineError::Interrupted` and
the engine stays usable. An interrupt requested while nothing runs has no effect on
the next operation. Like the budget, it cannot stop a blocked Rust host function.

An exhausted budget makes the script fault, not the whole operation fail: the
operation goes on with the next script and returns the first error, which names the
script. A script that runs out of its time or memory
`EngineOptions::max_script_faults` times (3 by default, 0 to disable) while an event,
a timer or a host answer is delivered to it is **quarantined**: its handlers and timers
no longer run, its pending host answers are dropped, and `Engine::quarantined_scripts`
lists it. `call` still reaches its exports. Reloading it with `load_script` replaces it
with a fresh script (unless its `ctx.hot.save` throws, which keeps the previous version:
unload it instead); unloading it also frees what it holds. An interrupt requested
through an `InterruptHandle` still stops the whole `emit`, `request` or
`advance_timers` at the handler or timer it reached, and is not a fault. An interrupt
that reaches a `ctx.hot.save` or `ctx.hot.dispose` callback also
surfaces as `EngineError::Interrupted`.

The memory limit is one pool for the whole engine: a script cannot be given a quota
inside it. See [Untrusted Mods](untrusted-mods.md) for what that means for code you do
not trust.

Deferred results are observed through `PendingCall`; only engine operations
execute JavaScript. `HostResolver::resolve` and `reject` enqueue responses from
any thread, but JavaScript resumes on the engine thread at `pump`. Timer
advancement is separate. A successful reload or unload cancels pending work
from the retired script generation: its `PendingCall` yields `EngineError::Cancelled`,
and late host replies for it are dropped. Failed reloads leave it running. Neither
`ctx.hot` nor a host reply migrates an awaiting stack to the new generation.

## Promises

Promise jobs queued by an operation run before it returns, for every script.
An `async` export resolves before synchronous `call` returns its value, and an
`async` request handler before synchronous `request` returns its reply. If their
Promise waits on a timer or host answer that requires a later operation, the
synchronous operation fails instead of waiting. Use `call_deferred` or
`request_deferred`: the Promise stays pending until `advance_timers` or `pump`
settles it. A rejection without a handler fails the operation that observes it;
a deferred result's rejection goes only to its `PendingCall` handle.

## Reload

A new script context and module graph are prepared before replacing the loaded
script. A failure at any step, from transpilation to top-level code, leaves the
previous version loaded with its state; the partially prepared graph is removed.
A successful reload resets script-local state.

This guarantees replacement of the script, not rollback of side effects: host
calls made during a failed initialization may already have changed the host.
Preparing both versions temporarily needs memory for both.

`reload_changed` detects changes through file sizes and modification times. A
change that keeps both identical (same size, within the filesystem's timestamp
resolution) is not seen; `load_project` always rereads every module whose stamp
changed and resolves again whenever the watched structure changed. The watched
files are the project's modules and their directories, its `tsconfig.json` and the
configs that one `extends` through a relative or absolute path (configs from
packages are not followed), the `package.json` files resolution read, and, after a
failed reload, the files that reload reached (a module it newly imported, say).

## Transpilation cache

The cache is off by default (`EngineOptions::cache_dir: None`). When enabled, cache
writes use temporary files in the cache directory and atomic replacement. A
checksum header detects truncated or modified contents; missing, older-format or
corrupt artifacts are rebuilt. This checksum is not authentication: keep cache
directories writable only by trusted users. Engines sharing a cache directory do
not see partially written artifacts. Atomic replacement is not a guarantee of
durability against every filesystem or power-loss failure.

Cache I/O never fails a load: an artifact that cannot be read is a miss and the
module is transpiled, and an artifact that cannot be written (full disk, read-only
directory) is skipped. Only creating the cache directory, in `Engine::new`, can fail.

Each artifact is one transpiled module and its source map, keyed by its source, its
file extension, and the compiler and crate versions. Import resolution is never
cached on disk. An engine reuses a project's resolutions in memory only while the
watched structure (directories, `tsconfig.json`, `package.json`, registered host
modules) is unchanged, so new files and configuration changes take effect on the
next load. Changing one module transpiles that module only.

The cache is content-addressed and never purged: every distinct version of a
module (and every compiler or crate version) adds an artifact, and nothing removes
old ones. A cache directory used during development grows with every edit; delete
the directory to reclaim the space, and the next loads rebuild what they need.
