# Rust And TypeScript Types

Rust types define the contract. `#[derive(TsSchema)]` generates the TypeScript
declaration and the two native converters for a type:

- `JsEncode`: Rust value → JavaScript value
- `JsDecode`: JavaScript value → Rust value

Values convert directly between Rust and QuickJS. No JSON text is produced.

## Reference Semantics

A value crosses the boundary exactly as if Rust had written it with `serde_json`
and JavaScript had read it with `JSON.parse`, and the reverse
(`JSON.stringify`, then `serde_json`). Serde attributes therefore change the
TypeScript shape and the runtime value the same way.

The native path is stricter than JSON text in three places:

1. Integers outside the JavaScript safe range, ±(2^53 − 1), fail on both sides.
   JSON would silently round them.
2. Non-finite `f32`/`f64` values cross as `NaN` and `±Infinity`. JSON would write
   `null`.
3. `NativeBytes` crosses as a `Uint8Array`.

Contract validation follows the native codec, not JSON text: it checks a
`Uint8Array` or `ArrayBuffer` as its bytes and accepts `NaN` and `±Infinity` as
numbers. Generated declarations quote property names that are not identifiers
(`"display-name": string`).

## Built-In Types

| Rust | TypeScript | Notes |
| --- | --- | --- |
| `bool` | `boolean` | Decoding requires a boolean. |
| `i8`…`i32`, `u8`…`u32` | `number` | Decoding requires an integral number within the Rust type's range. |
| `i64`, `u64`, `isize`, `usize`, `i128`, `u128` | `number` | Same rule, plus the ±(2^53 − 1) safe range. |
| `f32`, `f64` | `number` | `NaN`, `±Infinity` and `-0` are preserved. |
| `char` | `string` | Exactly one Unicode scalar. |
| `String`, `PathBuf`, `IpAddr`, `Ipv4Addr`, `Ipv6Addr` | `string` | Strings are copied (UTF-8 ↔ UTF-16). Lone surrogates are rejected, as in `serde_json`. |
| `()` | `null` | Decodes from `null` or `undefined`. |
| `Option<T>` | `T \| null` | `null` and `undefined` decode to `None`. |
| `Vec<T>`, `VecDeque<T>`, `Box<[T]>` | `T[]` | Decoding requires a JavaScript array. |
| `[T; N]` | `[T, T, …]` (N items) | Length must match exactly. |
| `HashSet<T, S>`, `BTreeSet<T>` | `T[]` | Duplicates collapse, as in serde. Any hasher `S`. |
| `HashMap<String, V, S>`, `BTreeMap<String, V>` | `Record<string, V>` | Plain object with own data properties (a `"__proto__"` key stays an ordinary key); properties `JSON.stringify` would drop are skipped. |
| `HashMap<{integer}, V, S>`, `BTreeMap<{integer}, V>` | `Record<number, V>` | Keys are canonical decimal strings in JavaScript. |
| `(A, B, …)` up to 12 elements | `[A, B, …]` | Length must match exactly. |
| `Box<T>`, `Arc<T>`, `Rc<T>` | `T` | Transparent. |
| `serde_json::Value` | JSON value | Integers still follow the safe-range rule. |
| `NativeBytes` | `Uint8Array` | See [Use Native Bytes](native-bytes.md). |

A map key must implement `MapKey`, a sealed trait implemented for `String` and the
primitive integers only.

### Feature-Gated Third-Party Types

Enable the matching Manivelle feature (`uuid`, `chrono`, `glam`) to use these types
natively, with the crate's own serde representation:

| Rust | TypeScript | Notes |
| --- | --- | --- |
| `uuid::Uuid` | `string` | Hyphenated lowercase on encode; decode accepts what uuid's serde impl reads from a string (simple, hyphenated, braced, URN, any case). |
| `chrono::DateTime<Tz>` | `string` | RFC 3339 like chrono's serde impl. Decodes into `DateTime<Utc>` (any offset, converted) and `DateTime<FixedOffset>`. |
| `chrono::NaiveDate`, `NaiveTime`, `NaiveDateTime` | `string` | ISO 8601 text, as chrono's serde impl writes it. |
| `glam::Vec2/3/4`, `DVec2/3/4`, `IVec2/3/4`, `UVec2/3/4`, `Quat` | tuple of N `number` | Components in order (`x, y, z, w`), following the `f32`/`f64`/`i32`/`u32` rules. |
| `glam::Mat2`, `Mat3`, `Mat4` | tuple of 4 / 9 / 16 `number` | Flat column-major array, like glam's serde impl. |

## Derived Types

```rust
#[derive(Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct Player {
    player_id: u32,
    position: (f32, f32),
    inventory: HashMap<String, Vec<Item>>,
    guild: Option<GuildId>,
}
```

| Rust shape | TypeScript |
| --- | --- |
| named struct | object |
| tuple struct | tuple |
| newtype struct, `#[serde(transparent)]` | the inner type |
| unit struct | `null` |
| unit-only enum | string union |
| enum with data | union following serde's tagging: external (default), `tag`, `tag` + `content`, `untagged` |

Mirrored serde attributes:

- containers: `rename_all` (also per direction), `rename_all_fields`, `tag`,
  `content`, `untagged`, `transparent`, `default`, `deny_unknown_fields`
- fields: `rename` (also per direction), `alias`, `skip`, `skip_serializing`,
  `skip_serializing_if`, `skip_deserializing`, `default`, `flatten`
- variants: `rename`, `alias`, `rename_all`, `skip`, `skip_serializing`,
  `skip_deserializing`

`flatten` merges a struct's fields into the surrounding object. A flattened
string-keyed map (`HashMap<String, V>`, `BTreeMap<String, V>`, `serde_json::Value`, or
an `Option` of one) receives every other key, so the object stays open:

```rust
#[derive(Serialize, Deserialize, TsSchema)]
struct Scores {
    player: String,
    #[serde(flatten)]
    rounds: BTreeMap<String, u32>,
}
```

```ts
type Scores = { player: string; [key: string]: number | string; };
```

The index signature also admits the declared fields' types, as TypeScript requires;
schema validation checks declared fields against their own types, every other key
against `V`, and strict validation accepts those extra keys. Flattening a value that
is neither a struct nor a string-keyed map is a compile error when the field type
shows it (`String`, `Vec<T>`, tuples, `HashMap<u32, V>` and other integer-keyed maps,
…), and a panic naming the problem when the schema is built otherwise: the type is
hidden behind an alias or a newtype, a merged field duplicates another, or two
flattened maps have different value types. The payload of an internally tagged
newtype variant, which serde merges the same way, follows the same rule.

Missing fields decode like serde: `Option<T>` becomes `None`, fields with
`default` take their default, anything else fails.

Errors name the failing path, for example `position.x: expected integer in u8
range, got 300` or `inventory[sword][2].id: missing field`.

### Directions

A derive macro cannot see which serde traits sit next to it, so
`#[derive(TsSchema)]` always generates both codecs. Restrict them on the type:

- `#[manivelle(encode_only)]`: only `JsEncode`
- `#[manivelle(decode_only)]`: only `JsDecode`
- `#[manivelle(schema_only)]`: TypeScript declaration only, no codec

If a field type supports only one direction, the compiler points at that field.

## Third-Party Types And The JSON Opt-In

The JSON reference path is never used silently. Serde attributes that only serde
can execute (`with`, `serialize_with`, `deserialize_with`, `from`, `into`,
`try_from`, `remote`, `bound`, …) are compile errors until you opt in:

```rust
#[derive(Serialize, Deserialize, TsSchema)]
struct Account {
    // Only this field goes through serde_json; the rest stays native.
    #[manivelle(codec = "json", type = "string")]
    id: some_crate::AccountId,

    // Native codec functions: `shout::encode_js(&value, ctx)` / `shout::decode_js(ctx, value)`.
    #[manivelle(with = "shout")]
    name: String,
}

// The whole type goes through serde_json.
#[derive(Serialize, Deserialize, TsSchema)]
#[serde(into = "String", try_from = "String")]
#[manivelle(codec = "json")]
struct Email(String);
```

`#[manivelle(type = "...")]` writes the TypeScript type as given and removes the need
for `TsSchema` on that field. Use it for types from crates that do not implement
`TsSchema`.

`#[manivelle(rename = "...")]` and `#[manivelle(optional)]` are only accepted on
`schema_only` types. On types with codecs, use the serde attributes, which change
the declaration and the runtime value together.

## Hand-Written Types

A type that cannot use the derive implements the traits directly:

```rust
use manivelle::js::{Ctx, Result, Value};
use manivelle::{JsDecode, JsEncode};

impl JsEncode for Celsius {
    fn encode_js<'js>(&self, ctx: &Ctx<'js>) -> Result<Value<'js>> {
        self.0.encode_js(ctx)
    }
}

impl JsDecode for Celsius {
    fn decode_js<'js>(ctx: &Ctx<'js>, value: Value<'js>) -> Result<Self> {
        f64::decode_js(ctx, value).map(Celsius)
    }
}
```

`manivelle::js` re-exports the QuickJS types these signatures use, at the exact version
Manivelle is built with. Use it instead of a direct `rquickjs` dependency; it also
provides `Runtime` and `Context` for unit-testing a codec.

`function` requires `Input: TsSchema + JsDecode` and
`Output: TsSchema + JsEncode`; `callback` requires
`Payload: TsSchema + JsEncode`.
