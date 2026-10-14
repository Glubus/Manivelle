# Migrate From `rustts`

Manivelle is `rustts` under a new name, restarted at 0.1.0. The API is the one of
`rustts` 0.4 development; `rustts` is deprecated and gets no further release.

Rename, in this order:

| `rustts` | `manivelle` |
| --- | --- |
| `rustts = "0.3"` in `Cargo.toml` | `manivelle = "0.1"` |
| `rustts_macros` | `manivelle_macros` (pulled in by the `derive` feature) |
| `use rustts::...` | `use manivelle::...` |
| `#[rustts(...)]` derive attributes | `#[manivelle(...)]` |
| `import ... from "rustts:env"` | `import ... from "manivelle:env"` |
| `rustts.d.ts`, `rustts.sdk.ts` (default SDK file names) | `manivelle.d.ts`, `manivelle.sdk.ts` |
| `rusttsSdk` (SDK export object) | `manivelleSdk` |
| `rustts-sdk` (binary) | `manivelle-sdk` |
| `RUSTTS_REQUIRE_TSC` | `MANIVELLE_REQUIRE_TSC` |

Existing transpile cache directories are rebuilt once: the artifact header changed.

The 0.3 to 0.4 API changes of `rustts` (typed registrations without the `typed_`
prefix, no JSON registration path, `HostFunctionSignature`, `HostContext::schema()`)
are not repeated here: code written against `rustts` 0.3 must follow the
[Register Host Functions And Callbacks](register-host-functions.md) guide.
