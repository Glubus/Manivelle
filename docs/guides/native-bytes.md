# Use Native Bytes

Use `NativeBytes` for byte payloads between Rust and TypeScript.

Without `NativeBytes`, byte buffers become arrays of numbers, which is expensive
for large payloads. `NativeBytes` crosses as a `Uint8Array` in both directions:

- Rust → JavaScript: zero-copy, immutable `Uint8Array` backed by the Rust bytes.
- JavaScript → Rust: a `Uint8Array`, an `ArrayBuffer`, or an array of integers
  in `0..=255`, copied once into Rust memory.

## Rust Contract

```rust
use serde::Deserialize;
use manivelle::{
    HostContract, HostContractKind, HostFunction, HostFunctionSignature, NativeBytes,
    TsSchema, VmError,
};

#[derive(Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct ReadAssetInput {
    path: String,
}

struct ReadAsset;

impl HostContract for ReadAsset {
    const NAME: &'static str = "asset.read";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for ReadAsset {
    type Input = ReadAssetInput;
    type Output = NativeBytes;
}

impl HostFunction for ReadAsset {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        let bytes = std::fs::read(input.path).map_err(VmError::from)?;
        Ok(NativeBytes::new(bytes))
    }
}
```

## TypeScript Shape

The generated declarations expose:

```ts
type NativeBytes = Uint8Array;
```

Scripts can use normal typed-array APIs:

```ts
const bytes = asset.read({ path: "payload.bin" });

if (bytes.byteLength >= 4) {
  const magic = bytes[0];
}
```

## Validation Caveat

When contract validation is enabled, each value is also converted to JSON for the
check, so a large byte payload is copied into a number array on every call. Keep
validation off for large byte payloads in production.

Use this type for byte payloads. Do not use JSON arrays for large buffers unless
you specifically need JSON compatibility.
