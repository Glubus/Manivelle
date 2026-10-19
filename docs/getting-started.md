# Getting Started

This guide creates a tiny Rust application that embeds TypeScript and exposes
one Rust function to the script.

The example contract is:

```ts
math.add({ left: 20, right: 22 }) // 42
```

## Create A Rust Project

```text
cargo new manivelle-hello
cd manivelle-hello
```

Add dependencies:

```text
cargo add manivelle@0.1 --features derive
cargo add serde --features derive
cargo add serde_json
```

## Create Your First Contract

Replace `src/main.rs` with the following (it is
[`examples/getting_started.rs`](https://github.com/Glubus/Manivelle/blob/main/examples/getting_started.rs),
which CI builds and runs):

```rust
use serde::{Deserialize, Serialize};
use serde_json::Value;
use manivelle::{
    Engine, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
    SdkFileNames, TsSchema, EngineError, EngineOptions,
};

#[derive(Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct AddInput {
    left: i32,
    right: i32,
}

#[derive(Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct AddOutput {
    value: i32,
}

struct Add;

impl HostContract for Add {
    const NAME: &'static str = "math.add";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for Add {
    type Input = AddInput;
    type Output = AddOutput;
}

impl HostFunction for Add {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(AddOutput {
            value: input.left + input.right,
        })
    }
}

fn main() -> Result<(), EngineError> {
    let mut engine = Engine::new(&EngineOptions::default())?;

    engine.registry().function::<Add>()?;
    // `manivelle init --from <this program>` asks for the declarations here.
    if engine.setup_protocol()?.exported {
        return Ok(());
    }
    engine.registry().write_sdk_files_with_names(
        "target/generated",
        &SdkFileNames {
            types: "my_sdk.d.ts".into(),
            sdk: "my_sdk.ts".into(),
        },
    )?;

    engine.load_script("math-script", include_str!("math_script.ts"))?;

    let output: Value = engine.call("math-script", "run", ())?;
    println!("{output}");

    Ok(())
}
```

What matters:

- `HostContract::NAME = "math.add"` controls the TypeScript API path.
- `AddInput` is the script input type.
- `AddOutput` is the script output type.
- `#[derive(TsSchema)]` lets the registry generate TypeScript declarations.
- `function::<Add>()` registers the Rust function in the engine; register
  contracts before loading the scripts that call them.
- `Engine` runs the script on the current thread: `call` returns once the script
  function has returned.

## Add The TypeScript Script

Create `src/math_script.ts`:

```ts
export function run() {
  const result = math.add({ left: 20, right: 22 });
  return result.value;
}
```

Run it:

```text
cargo run
```

Expected output:

```text
42
```

## Generated Files

This line:

```rust
engine.registry().write_sdk_files_with_names(
    "target/generated",
    &SdkFileNames {
        types: "my_sdk.d.ts".into(),
        sdk: "my_sdk.ts".into(),
    },
)?;
```

writes:

```text
target/generated/
  my_sdk.d.ts
  my_sdk.ts
```

`my_sdk.d.ts` contains the TypeScript declarations for your contracts. In this
example, it declares that the global `math.add(...)` accepts `{ left, right }` and
returns `{ value }`. Include it in the scripts' `tsconfig.json` so `tsc --noEmit`
type-checks them: loading a script transpiles it without type checking.

`my_sdk.ts` is the generated SDK module, with typed helpers built on the same
host functions. A host application can generate these files on demand through
its own CLI.

For a real application package, you can also expose imports such as:

```ts
import { math } from "my_sdk";
```

That requires setting `HostContract::IMPORT_MODULE = "my_sdk"` and
`HostContract::EXPORT_PATH = &["math", "add"]` on the Rust contract. See
[Generate TypeScript SDK Files](guides/generate-sdk-files.md).

## For Mod Authors

The line with `setup_protocol` lets people who write scripts for your program
get its declarations without access to your source: they run
`manivelle init my-mod --from path/to/your-program` and get a type-checking project.
See [Generate TypeScript SDK Files](guides/generate-sdk-files.md#give-mod-authors-the-declarations-manivelle-init).

## Next

Read [Register Host Functions And Callbacks](guides/register-host-functions.md)
when you want callbacks/events, friendlier SDK namespaces, and stricter
validation.
