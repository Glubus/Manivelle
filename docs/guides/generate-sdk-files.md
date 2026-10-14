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

## What The Declaration File Contains

The declaration file contains the public TypeScript API:

- host function input and output types
- callback payload types
- the `ctx` global: `ctx.hot` always (see
  [Keep State Across Reloads](engine.md#keep-state-across-reloads)), and
  `ctx.on(...)` once a callback is registered
- generated namespaces such as `user.find(...)`
- event maps for typed callback subscriptions

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

## Application Tooling

Most applications should wrap this in their own CLI. A game SDK might expose:

```text
my-sdk init my-mod
my-sdk generate-sdk
my-sdk check
my-sdk build
```

That CLI can call `write_sdk_files_with_names(...)`, create `tsconfig.json`, and
decide where generated files live. `Manivelle` only owns the
registry-to-TypeScript generation.
