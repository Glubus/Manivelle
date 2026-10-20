# Register Host Functions And Callbacks

This guide shows the normal flow:

1. Create an `Engine`.
2. Declare Rust payload structs.
3. Derive `TsSchema`.
4. Implement `HostFunction` or `HostCallback`.
5. Register contracts with `function` and `callback`.
6. Generate declaration and SDK files for your package.
7. Load scripts that call your functions and handle your events.

## Install With Derive Support

```toml
[dependencies]
manivelle = { version = "0.1", features = ["derive"] }
serde = { version = "1", features = ["derive"] }
```

## Create The Engine

```rust
use manivelle::{Engine, EngineError, EngineOptions};

fn create_engine() -> Result<Engine, EngineError> {
    Engine::new(&EngineOptions {
        cache_dir: Some("target/manivelle-cache".into()),
        ..EngineOptions::default()
    })
}
```

The cache directory stores transpiled JavaScript artifacts, so reloads and later
runs reuse them. Without it (the default), every load transpiles in memory.

## Declare A Host Function

Host functions are Rust calls exposed to TypeScript. The contract name controls
the generated TypeScript namespace. For example, `user.find` becomes
`user.find(...)`.

```rust
use serde::{Deserialize, Serialize};
use manivelle::{
    HostContract, HostContractKind, HostFunction, HostFunctionSignature, TsSchema, EngineError,
};

#[derive(Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct FindUserInput {
    user_id: u64,
    include_roles: bool,
}

#[derive(Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct FindUserOutput {
    user_id: u64,
    display_name: String,
    active: bool,
    roles: Vec<String>,
}

struct FindUser;

impl HostContract for FindUser {
    const NAME: &'static str = "user.find";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindUser {
    type Input = FindUserInput;
    type Output = FindUserOutput;
}

impl HostFunction for FindUser {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(FindUserOutput {
            user_id: input.user_id,
            display_name: format!("user-{}", input.user_id),
            active: true,
            roles: if input.include_roles {
                vec!["admin".into(), "editor".into()]
            } else {
                Vec::new()
            },
        })
    }
}
```

`function::<FindUser>()` uses `FindUserInput::schema()` and
`FindUserOutput::schema()` automatically, so you do not need to hand-write the
TypeScript shape.

`HostFunctionSignature` is what scripts see: the input and output types.
`HostFunction` adds the static `call` that implements it.

### Shortcut: `#[host_fn]`

With the `derive` feature, the three impls above reduce to the function itself.
The marker type is named after the function in `PascalCase` (`find_user` → `FindUser`):

```rust
use manivelle::{EngineError, host_fn};

#[host_fn(name = "user.find")]
fn find_user(input: FindUserInput) -> Result<FindUserOutput, EngineError> {
    Ok(FindUserOutput { user_id: input.user_id, display_name: format!("user-{}", input.user_id), active: true, roles: vec![] })
}

// registry.function::<FindUser>()?;
```

The function takes zero or one parameter (`Input` is `()` without one), and cannot be
`async` or generic. It returns `Result<Output, EngineError>`, or a plain `Output` (or
nothing) when it cannot fail: only a return type named `Result` is read as fallible.
Closures and `async_function_with` keep the manual form.

The `///` comments of the function become the JSDoc of the generated TypeScript
declarations and SDK, so a script author sees them when hovering `user.find` in an
editor. With the manual form, set `HostContract::DOCS` to the same text.

### Group Functions Under A Namespace: `#[host_mod]`

`#[host_mod]` is optional. On an inline module, it prefixes the contract names of its
`#[host_fn]` functions, so `name` can be left out:

```rust
use manivelle::{EngineError, host_fn, host_mod};

#[host_mod(namespace = "math")] // or `#[host_mod]`: uses the module's name
mod math {
    use super::*;

    #[host_fn]                                   // "math.add"
    pub fn add(input: AddInput) -> Result<AddOutput, EngineError> { /* ... */ }

    #[host_fn(name = "debug.log", no_namespace)] // "debug.log", not prefixed
    pub fn log(input: LogInput) -> Result<(), EngineError> { /* ... */ }
}

// registry.function::<math::Add>()?;
```

Use `pub fn` to register the marker from outside the module. Only the module's own
functions are rewritten: a nested module needs its own `#[host_mod]`. `mod name;` in
another file is not supported, because a macro cannot read it.

### Receive The Calling Script: `#[host_fn_with_ctx]`

A static `HostFunction` cannot know which script called it. A function whose first
parameter is the `Caller` is declared with `#[host_fn_with_ctx]` and registers itself:

```rust
use manivelle::{Caller, EngineError, host_fn_with_ctx};

#[host_fn_with_ctx(name = "user.find")]
fn find_user(ctx: &Caller<'_>, input: FindUserInput) -> Result<FindUserOutput, EngineError> {
    Ok(FindUserOutput { display_name: format!("{} asks {}", ctx.script_id(), input.user_id) })
}

FindUser::register(engine.registry())?;
```

The marker has `HostContract` and `HostFunctionSignature` but no `HostFunction`; its
`register` calls `function_with_caller`. The input is optional (`fn(ctx)`).

### Settle Later: `#[host_fn_async]`

An async contract takes a `HostResolver<Output>` as its last parameter, and the `Caller`
first if it wants it. Keep the resolver where the game can settle it:

```rust
use manivelle::{HostResolver, host_fn_async};

#[host_fn_async(name = "scores.lookup")]
fn lookup_score(player: String, resolver: HostResolver<u32>) {
    queue().push((player, resolver));
}

LookupScore::register(engine.registry())?;
```

`register` uses `async_function_with`, or `async_function_with_caller` with a `ctx`.
Both macros also work inside `#[host_mod]`.

## Implement A Host Function With A Closure

A contract that only implements `HostFunctionSignature` is registered together
with its handler, a closure that can hold state. Here `SpawnEnemy` is declared like
`FindUser` above, without the `HostFunction` impl, and `world` is a
`Send + Sync` handle to game state:

```rust
engine
    .registry()
    .function_with::<SpawnEnemy>(move |input| world.spawn(input.kind, input.position))?;
```

The generated TypeScript is the same as for a static function. The closure must be
`Send + Sync + 'static` because the registry is; `function_with` is the closure
counterpart of `function`.

### Return A Promise Resolved By The Game

Use `async_function_with` when a host answer arrives in a later frame.
The contract only implements `HostFunctionSignature`; its `Output` is the
**resolved value**, and the generated TypeScript signature returns
`Promise<Output>`. Keep the resolver in game state and settle it when the data
arrives:

```rust
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use manivelle::HostResolver;

let pending: Arc<Mutex<VecDeque<HostResolver<u32>>>> = Arc::new(Mutex::new(VecDeque::new()));
let next = Arc::clone(&pending);
engine.registry().async_function_with::<LookupScore>(
    move |_player, resolver| {
        next.lock().expect("score queue").push_back(resolver);
        Ok(())
    },
)?;

// Later, on the host's schedule (another thread may enqueue this):
if let Some(resolver) = pending.lock().expect("score queue").pop_front() {
    resolver.resolve(42)?;
}
engine.pump()?; // only now do awaiting script continuations run
```

`LookupScore` declares `Input` and `Output = u32` like `FindUser` above.
`async_function_with` takes its schemas from `TsSchema` of `Input` and `Output`
(`Output` must be `Send`); `async_function_with_caller` also receives a
`Caller`. Dropping a resolver rejects the Promise; `reject(error)`
does so explicitly. An old resolver becomes cancelled when its script reloads
successfully or unloads, and `resolve`/`reject` then return
`EngineError::Cancelled`. No host reply automatically drives JavaScript.

If an exported function awaits this reply, start it with `call_deferred`:
the synchronous `call` cannot wait for a host reply from a later frame.

### Know The Calling Script

`function_with_caller` hands the handler a
`Caller` too: `caller.script_id()` is the id the calling script was loaded under,
kept across reloads. Use it to scope a mod's permissions, storage or logs:

```rust
engine
    .registry()
    .function_with_caller::<SaveSetting>(move |caller, input| {
        settings.save(caller.script_id(), &input.key, input.value)
    })?;
```

### Reach The Host's State: `with_state`

A handler is a `Send + Sync + 'static` closure, so it cannot capture a `&mut` the host
owns, such as a game world. `Engine::with_state` lends one for the duration of a closure,
and a handler that receives the `Caller` reads it:

```rust
struct World { score: u32 }

engine
    .registry()
    .function_with_caller::<AddScore>(|caller, input| {
        caller.with_state(|world: &mut World| {
            world.score += input.amount;
            AddScoreOutput { total: world.score }
        })
    })?;

let mut world = World { score: 0 };
engine.with_state(&mut world, |engine| {
    engine.advance_timers(dt)?;
    engine.emit("tick", &tick)?;
    engine.pump()
})??;
```

Every call that can enter a script (`call`, `emit`, `request`, `advance_timers`, `pump`,
`load_script`, `reload_changed`) can run inside the closure; none of them changes. The
`&mut` stays borrowed by `with_state` until it returns, so the host cannot touch the state
while scripts run, and a handler cannot keep it afterwards.

`Caller::with_state` takes a closure, never hands the `&mut` out, and returns an error
where a second `&mut` would otherwise exist:

| Error | When |
| --- | --- |
| `EngineError::NoState` | the call did not run inside `with_state`, for example a timer advanced outside a scope. The script sees a JavaScript error it can catch. |
| `EngineError::StateType` | the scope lent another type than the one asked for. |
| `EngineError::StateBusy` | a handler up the call stack already holds the state. |
| `EngineError::StateScopeActive` | returned by `with_state` itself when called inside a scope: scopes do not nest. |

Handlers without a `Caller` do not see the state.

## Errors And Panics In A Host Function

A handler that returns `Err(EngineError)` throws a JavaScript error in the script, which can
catch it. A handler that **panics** panics out of the engine call that reached it
(`Engine::call`, `emit`, `advance_timers`, ...), after the operation's Promise jobs ran.
The same holds for `async` handlers and for a `console` sink:
a script's `try`/`catch` cannot swallow a Rust panic, and the engine stays usable if you
catch the unwind. With `panic = "abort"` the process aborts, as for any Rust code.

## Declare A Callback

Callbacks are events the host emits to scripts. The contract name is the event
name scripts subscribe to with `ctx.on("user.found", ...)`; the generated SDK adds
the ergonomic alias `user.onFound(...)`.

```rust
use serde::{Deserialize, Serialize};
use manivelle::{HostCallback, HostContract, HostContractKind, TsSchema};

#[derive(Deserialize, Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct UserFoundPayload {
    user_id: u64,
    display_name: String,
    roles: Vec<String>,
}

struct UserFound;

impl HostContract for UserFound {
    const NAME: &'static str = "user.found";

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for UserFound {
    type Payload = UserFoundPayload;
}
```

### Shortcut: `#[host_event]`

With the `derive` feature, the attribute on the payload type replaces the marker and its two
impls. The type is its own payload, and its `///` comments become the JSDoc of the event:

```rust
use manivelle::{TsSchema, host_event};

/// A user was found.
#[host_event(name = "user.found")]
#[derive(Deserialize, Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct UserFound {
    user_id: u64,
    display_name: String,
}

engine.registry().callback::<UserFound>()?;
engine.emit(UserFound::NAME, &UserFound { user_id: 7, display_name: "Ada".into() })?;
```

`reply = Type` makes it a request (`#[host_event(name = "menu.label", reply = String)]`),
and `#[host_mod]` prefixes the name of an event like that of a function. The type cannot
be generic.

## Declare A Request

A request is a callback whose handlers answer: `Engine::request` returns what each
handler returned. Declare the callback like `UserFound`, here `MenuLabel` named
`menu.label` with a `String` payload, implement `HostRequest` on top of
`HostCallback`, and register it with `request` so the generated TypeScript
types the handlers' return value:

```rust
use manivelle::HostRequest;

impl HostRequest for MenuLabel {
    type Reply = String;
}

engine.registry().request::<MenuLabel>()?;

let labels: Vec<(ScriptId, String)> = engine.request("menu.label", "save")?;
```

Each reply comes with the id of the script that gave it. An `async` handler's
Promise is awaited; see [Requests](engine.md#requests).

## Register Contracts In The Registry

Register contracts in the engine's registry before loading the scripts that use
them:

```rust
let mut engine = create_engine()?;

engine
    .registry()
    .function::<FindUser>()?
    .callback::<UserFound>()?;
```

That registry is the source of truth for:

- the host functions installed in every script
- generated `.d.ts` declarations
- generated TypeScript SDK helpers
- the cache key of transpiled scripts

The synchronous host functions above run on the engine's thread and return
their value directly. An `Err` returned by the handler is thrown in the script
as an exception the script can catch. To return a Promise instead, use
[`async_function_with`](#return-a-promise-resolved-by-the-game).

## Generate The SDK Files

```rust
use manivelle::SdkFileNames;

let written = engine.registry().write_sdk_files_with_names(
    "target/generated",
    &SdkFileNames {
        types: "my_sdk.d.ts".into(),
        sdk: "my_sdk.ts".into(),
    },
)?;

assert!(written.types_path.ends_with("my_sdk.d.ts"));
assert!(written.sdk_path.ends_with("my_sdk.ts"));
```

The generated SDK is a TypeScript module that exposes:

```ts
user.find({ userId: 7, includeRoles: true });

ctx.on("user.found", event => {
  event.displayName.toUpperCase();
});

events.user.found(event => {
  event.roles.length.toFixed();
});

user.onFound(event => {
  event.roles.map(role => role.toUpperCase());
});
```

For object schemas, the SDK also emits lightweight helpers:

```ts
const input = models.FindUserInput.create({
  userId: 7,
  includeRoles: true,
});

if (FindUserInputModel.is(input)) {
  const wrapped = FindUserInputModel.wrap(input);
  wrapped.toJSON().userId.toFixed();
}
```

## Load And Run A Script

Scripts reach host functions through namespaced globals such as `user.find(...)`
and subscribe to events with `ctx.on(...)`, without an import:

```ts
let lastFound = "none";

ctx.on("user.found", event => {
  lastFound = `${event.displayName}:${event.roles.join(",")}`;
});

export function lookup() {
  const result = user.find({ userId: 7, includeRoles: true });
  return `${result.displayName}:${result.roles.length}:${result.active}`;
}

export function observed() {
  return lastFound;
}
```

Rust loads the script, calls its exports and emits events to it:

```rust
engine.load_script("user-rules", include_str!("user_rules.ts"))?;

let result: String = engine.call("user-rules", "lookup", ())?;
assert_eq!(result, "user-7:2:true");

engine.emit(
    UserFound::NAME,
    &UserFoundPayload {
        user_id: 7,
        display_name: "user-7".into(),
        roles: vec!["admin".into(), "editor".into()],
    },
)?;

let observed: String = engine.call("user-rules", "observed", ())?;
assert_eq!(observed, "user-7:admin,editor");
```

To use the SDK helpers (`user.onFound`, `events`, `models`) in an inline script,
load the SDK source followed by the script:

```rust
let sdk = engine.registry().sdk()?;
engine.load_script("user-rules", &format!("{sdk}\n{}", include_str!("user_rules.ts")))?;
```

A project can import the generated SDK file instead, for example
`import { user } from "./generated/my_sdk";`.

## Validation Policy

Runtime contract validation is configurable. The default is permissive. Use
strict unknown-field rejection when you want scripts to fail fast on extra input
fields.

```rust
use manivelle::{ContractValidation, UnknownFieldValidation, EngineOptions};

let mut options = EngineOptions::default();
options.contract_validation = ContractValidation::Inputs;
options.unknown_field_validation = UnknownFieldValidation::Reject;
```

That is useful for SDK development, tests, and modding APIs where clear error
messages matter more than accepting loose payloads.

## What To Remember

- Register contracts before loading scripts.
- Every payload type implements `TsSchema`, `JsDecode` and `JsEncode`: derive them,
  or use `serde_json::Value` for a free-form value. Use the `_with` variants when the
  handler needs state, and the `_with_caller` ones when it needs to know which script
  called.
- Keep event names stable; they become part of the generated TypeScript API.
- Use `user.onFound(...)`-style aliases for friendly game SDKs, and keep
  `ctx.on("user.found", ...)` / `ctx.off(...)` available for low-level dynamic cases.
