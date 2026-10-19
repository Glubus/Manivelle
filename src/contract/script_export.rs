//! Typed names of the functions a script exports.

use super::arity::for_each_tuple;
use super::{JsArgs, JsDecode, Schema, TsSchema};

/// A function a script exports, declared once in Rust so that calling it is checked by
/// the compiler: the export's name, its arguments and what it returns.
///
/// ```ignore
/// struct Run;
///
/// impl ScriptExport for Run {
///     const NAME: &'static str = "run";
///     type Args = (Tick,);
///     type Output = Forecast;
/// }
///
/// let forecast = engine.call_typed::<Run>("weather-rules", (tick,))?;
/// ```
///
/// The compiler checks the Rust side only: a script still has to export `NAME` with a
/// compatible signature, which [`Engine::call_typed`](crate::Engine::call_typed)
/// reports at the call when it does not. To have `tsc` check the script too, register the
/// export with [`script_export`](crate::InMemoryHostContractRegistry::script_export): the
/// generated declarations then hold a `ScriptExports` type that a script annotates its
/// exports with.
pub trait ScriptExport {
    /// The name of the exported function.
    const NAME: &'static str;

    /// The arguments of the call: `()`, or a tuple with one element per argument.
    type Args: JsArgs;

    /// What the export returns; an `async` export resolves before it is decoded.
    type Output: JsDecode;
}

/// The TypeScript types of a [`ScriptExport`]'s argument list: nothing for `()`, one per
/// element of a tuple, each given by [`TsSchema`].
pub trait TsArgs {
    /// The schema of each argument, in order.
    fn ts_schemas() -> Vec<Schema>;
}

impl TsArgs for () {
    fn ts_schemas() -> Vec<Schema> {
        Vec::new()
    }
}

macro_rules! tuple_ts_args {
    ($len:literal => $($index:tt $name:ident),+) => {
        impl<$($name: TsSchema),+> TsArgs for ($($name,)+) {
            fn ts_schemas() -> Vec<Schema> {
                vec![$($name::schema()),+]
            }
        }
    };
}

for_each_tuple!(tuple_ts_args);
