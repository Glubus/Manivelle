//! The integration tests: one binary, so the crate links once and `support` compiles once.
//! Run one file with `cargo test --test suite <module>`, e.g. `cargo test --test suite engine_scripts`.

mod cases;
mod support;
