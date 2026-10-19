# Packages And The `manivelle` Tool

A package is a script project with a `manivelle.kdl` manifest that says what it is, what it
needs and which program it runs in. The `manivelle` tool reads it to start that
program, the **target**, with the package, or without any package.

**The tool is optional.** Packages are loaded by the library: a program that calls
`engine.load_packages(&[dir])` loads them, checks them and enforces their permissions
without the `manivelle` command, and that is how a shipped game loads the mods of its
players. The tool is for the people who write packages: `init` sets a project up, `run`
starts the target with it, `check` validates it.

## The Manifest

```kdl
package "better-combat" {
    version "1.2.0"
    author "dev"

    engine {
        requires ">=0.1.0"
    }

    depends {
        "core" version "^1.0"
    }

    permissions {
        world.modify
        entity.spawn
    }

    entry "src/index.ts"
    target "../game"
}
```

The manifest is [KDL 2](https://kdl.dev): a node is a name, arguments (`"quoted"` or bare
words) and an optional `{ ... }` block, and `//`, `/* */` and `/-` comment. Only string
values are used (quote versions and paths), with no `key=value` properties. Unknown nodes
are errors, so a misspelled key cannot silently do nothing.

| Node | Meaning |
| --- | --- |
| `package "name"` | The package's name, also its script id. Letters, digits, `_`, `-`, `.`. |
| `version` | Required, [semver](https://semver.org). |
| `author` | Optional. |
| `engine { requires "..." }` | The Manivelle versions the package works with. |
| `depends { "name" version "req" }` | Packages loaded before this one; the version is optional. |
| `permissions { ... }` | The host contracts the package may use: `world.modify`, `world.*` (a namespace) or `*`. See [Permissions](#permissions). |
| `entry` | Required: the entry file, relative to the manifest, inside its directory. |
| `target` | The program `manivelle run` starts, relative to the manifest. |

## Prepare The Target

The target calls `setup_protocol` once at startup and `poll_protocol` once per frame:

```rust
let mut engine = Engine::new(&EngineOptions::default())?;
engine.registry().function::<FindUser>()?;

if engine.setup_protocol()?.exported {
    return Ok(()); // `manivelle init` asked for the SDK; it is written
}
loop {
    engine.poll_protocol(); // answers the tool; one atomic read when idle
    engine.pump()?;
    // ... the frame
}
```

Started by hand, a program has none of the environment variables below and
`setup_protocol` does nothing. A shipped program that wants packages calls
`engine.load_packages(&[dir])` itself.

## `manivelle run`

```text
manivelle run                      # the package in ., with its target
manivelle run --no-packages            # the same target, vanilla
manivelle run --packages ../packages       # also load the packages in ../packages
manivelle run --watch              # reload package files when they change
manivelle run --target ./game -- --headless
```

The tool reads `manivelle.kdl`, starts the target and gives it the package (plus any `--packages`
directory, each a package or a directory of packages) to load. The target's own `setup_protocol`
loads them, and the tool prints what loaded and what failed. `--no-packages` can be used
without a manifest when `--target` is given. `--watch` asks the target once a second to
reload the files that changed, as `Engine::reload_changed` does.

### Loading rules

`Engine::load_packages` reads every manifest and checks the whole set before loading any package:
names are unique, `engine` requirements are met, each dependency is there in an accepted
version, and there is no cycle. A set that cannot work together loads nothing. Packages then
load dependencies first, each as its own project with a context of its own. A package that
fails to load does not stop the others, except the packages that depend on it.

## Permissions

A package reaches only the host contracts it lists: the host functions it can call and
the events it can listen to. A permission is a contract name (`world.modify`), a
namespace with a wildcard (`world.*`), or `*` for all of them.

```kdl
permissions {
    math.add
    events.*
}
```

A function the package may not use is still there, so its types hold, and calling it
throws `permission denied: package "x" needs "world.modify" in the permissions of its
manivelle.kdl`. `ctx.on` of an event it may not listen to throws the same. A package that
lists no permission reaches no host contract at all.

Only packages are restricted: a script the target loads with `load_script` or
`load_project` is trusted. Permissions limit what a package can ask of the target; they
do not isolate untrusted code from the process, see [Untrusted Mods](untrusted-mods.md).

## How They Talk

The tool listens on `127.0.0.1` and starts the target with three environment variables:
`MANIVELLE_PROTOCOL` (the address), `MANIVELLE_TOKEN` (a random secret the target must
show in its first message) and `MANIVELLE_PACKAGES` (the directories to load, separated like
`PATH`; unset with `--no-packages`). Messages are JSON lines, defined in
`manivelle::protocol`. Anything that connects without the token is dropped.

The connection starts one thread in the target. It only reads requests and queues them;
it never touches the engine. The requests run on the engine's own thread in
`poll_protocol`: export the SDK, list contracts, list packages, reload. A target that never
calls `poll_protocol` still starts with its packages, but does not answer later requests.

While attached, `console.log` and the other `console` calls of the scripts are sent to
the tool, which prints them (warnings and errors on stderr). The program's own stderr
output of them is silenced meanwhile, so a line shows once; a sink the program chose with
`set_console` still receives every line. The lines leave on a thread of their own and
are flushed when the engine is dropped; a program that ends with `std::process::exit`
without dropping it may lose the last ones.

## `manivelle init` And `manivelle check`

`manivelle init my-package --from ./game` asks the program for its API and sets the
project up: the declarations in `manivelle/`, a `tsconfig.json`, `src/main.ts` and a
`manivelle.kdl` named after the directory, with the program as its `target` and every host
function and event of the program listed, commented out, under `permissions`. Uncomment
what the package needs.

`manivelle check` validates packages without loading them into a program, and **reads
their TypeScript** against the program's contracts:

```text
manivelle check                          # the package in .: manifest, entry file, dependencies
manivelle check --packages ../mods       # also the packages in ../mods, as a set
manivelle check --from ./game            # also: the code against the program's contracts
manivelle check --descriptors api.json   # the same, from a descriptors file (no program started)
manivelle check --from ./game --fix      # also: add the permissions the code uses and lacks
```

With the program's contracts it reports, for the files of the package (the project the
entry imports, not the generated SDK file):

| Finding | Level |
| --- | --- |
| a permission that names no contract (`world.modfy`, with `did you mean "world.modify"?`) | error |
| the code uses a host function or listens to an event the permissions do not list | error |
| `math.ad(...)`: a chain under a host namespace that is no host function (with the closest name) | error |
| `ctx.on("user.fond")`: an event the program does not have | error |
| a permission the code never uses | warning |
| the program calls a `script_export` function on scripts that the entry does not export | warning |
| `math[name](...)` or `ctx.on(name, ...)`: the code hides which contract it reaches | warning |

Each finding names the file and line. `--fix` adds the permissions the code uses and the
manifest of DIRECTORY lacks, after the ones already there (never removing one, and keeping
comments), then checks again. `--from` starts the program once, which means trusting it.

The reading is **static and best effort**. It follows `math.add(...)`, `ctx.on("event", ...)`,
the same through the generated SDK (`import { math } from "./manivelle.sdk"`) and the
`manivelle:env` module, and it tells a host namespace from a local variable of the same
name. It cannot see a name built at runtime, which is why it warns about computed access.
A clean report is not a proof; the permissions the engine enforces at runtime are what
keeps a package in bounds.

The same checks are a library call, for a `build.rs` or a test of your own, without the
tool: `check_package(&manifest, &registry.descriptors()?)` returns the findings, and
`analyze_project` the raw usage.

## Limits

- A script loaded with `load_script` or `load_project` has no package and no restriction.
- A root is a package, or a directory of packages: packages nested deeper are not searched.
