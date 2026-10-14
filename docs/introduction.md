# Manivelle

`Manivelle` is an embeddable TypeScript scripting layer for Rust applications and
games. The host exposes typed Rust functions and events to scripts, runs the
scripts on its own thread through `Engine`, and generates the scripts' TypeScript
declarations and SDK from the Rust contracts.

The short version:

- Rust owns the truth: host functions, callbacks, and context metadata are
  declared in Rust.
- Scripts call host functions such as `user.find(...)` directly; each call runs
  the Rust handler and returns its value.
- Scripts subscribe to host events with `ctx.on("user.found", ...)`, or through
  the generated SDK's event wrappers such as `events.user.found(...)` and friendly
  domain aliases such as `user.onFound(...)`.
- The generated files are regular TypeScript. The host application chooses the
  file names, for example `my_sdk.d.ts` and `my_sdk.ts`.

## What You Can Build

Game UI, gameplay rules, mods, editor and tool automation, simulation rules, or
plugin-like systems where the host is Rust and scripts are TypeScript.

Example script:

```ts
let lastUser = "none";

ctx.on("user.found", event => {
  lastUser = `${event.displayName}:${event.roles.join(",")}`;
});

export function lookup() {
  const result = user.find({ userId: 7, includeRoles: true });
  return `${result.displayName}:${result.roles.length}`;
}
```

## What It Is Not

Manivelle is not a server runtime and not a Node or Deno replacement:

- it starts no threads and has no event loop, network or filesystem API;
  scripts run only when the host calls them, and reach the outside world only
  through the host functions you register
- an `Engine` stays on the thread that created it
- imports must be statically discoverable; dynamic `import(...)` is rejected
- synchronous calls settle script-only Promise jobs before returning; host or
  timer-driven async exports use `call_deferred` / `request_deferred` and
  `pump` / `advance_timers`
- `HostContext` is declarative metadata only

Start with [Getting Started](getting-started.md), then
[Run Scripts With `Engine`](guides/engine.md).
