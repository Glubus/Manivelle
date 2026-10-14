//! Minimal host context contract trait.

use super::{HostContract, Schema};

/// Minimal declarative host context contract.
///
/// A context is contract metadata only: schema, ABI seed input,
/// declaration output, and optional generated SDK typing. It does not install a
/// mutable Rust object graph into every QuickJS context and does not create a
/// host-call bridge by itself.
pub trait HostContext: HostContract {
    /// Schema of the context object scripts see, rendered in the generated declarations.
    fn schema() -> Schema;
}
