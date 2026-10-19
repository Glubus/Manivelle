//! Rust type to TypeScript schema bridge.

use std::{
    cell::RefCell,
    collections::{BTreeMap, BTreeSet, HashMap, HashSet, VecDeque},
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
    path::PathBuf,
    rc::Rc,
    sync::Arc,
};

mod object;

use super::arity::for_each_tuple;
use super::{Schema, TsRecordKey, TsType};

pub use object::ObjectSchema;

thread_local! {
    static SCHEMA_DEPENDENCY_STACK: RefCell<Vec<&'static str>> = const { RefCell::new(Vec::new()) };
}

/// Contract schema emitted by a Rust type.
///
/// Future derive/proc-macro support should target this trait. Runtime host contracts
/// can then use `T::schema()` instead of hand-building [`Schema`] values.
pub trait TsSchema {
    /// Stable TypeScript schema name, or `None` for an anonymous type.
    ///
    /// Primitive and container implementations are anonymous, so the declaration renderer
    /// emits their inline type shape instead of a top-level alias.
    fn schema_name() -> Option<&'static str> {
        None
    }

    /// TypeScript type shape represented by this Rust type.
    fn ts_type() -> TsType;

    /// Named schemas referenced by this Rust type.
    fn schema_dependencies() -> Vec<Schema> {
        Vec::new()
    }

    /// Full schema metadata for this Rust type.
    fn schema() -> Schema {
        let schema = match Self::schema_name() {
            Some(name) => Schema::typed(name, Self::ts_type()),
            None => Schema::anonymous(Self::ts_type()),
        };
        schema.with_dependencies(Self::schema_dependencies())
    }

    /// Validates one JSON value against this type's generated schema.
    ///
    /// Unknown object fields are allowed, matching the default host bridge
    /// validation policy.
    ///
    /// # Errors
    ///
    /// Returns a human-readable validation path and reason when the value does
    /// not match this schema.
    fn validate_json(value: &serde_json::Value) -> Result<(), String> {
        Self::schema().validate_json(value)
    }

    /// Validates one JSON value against this type's generated schema and rejects unknown fields.
    ///
    /// # Errors
    ///
    /// Returns a human-readable validation path and reason when the value does
    /// not match this schema or contains undeclared object fields.
    fn validate_json_strict(value: &serde_json::Value) -> Result<(), String> {
        Self::schema().validate_json_strict(value)
    }
}

/// Pushes the named schema for `T`, plus its own dependencies, when `T` has a stable name.
///
/// This helper is public for derive macro expansion but is not part of the intended
/// high-level embedding API.
#[doc(hidden)]
pub fn push_schema_dependency<T>(dependencies: &mut Vec<Schema>)
where
    T: TsSchema,
{
    let _guard = match SchemaDependencyGuard::enter::<T>() {
        DependencyGuardState::Entered(guard) => Some(guard),
        DependencyGuardState::Unnamed => None,
        DependencyGuardState::Recursive => return,
    };

    dependencies.extend(T::schema_dependencies());

    let schema = T::schema();
    if schema.declared_name().is_some() {
        dependencies.push(schema);
    }
}

enum DependencyGuardState {
    Entered(SchemaDependencyGuard),
    Unnamed,
    Recursive,
}

struct SchemaDependencyGuard {
    name: &'static str,
}

impl SchemaDependencyGuard {
    fn enter<T>() -> DependencyGuardState
    where
        T: TsSchema,
    {
        let Some(name) = declared_schema_name(T::schema_name()) else {
            return DependencyGuardState::Unnamed;
        };

        let entered = SCHEMA_DEPENDENCY_STACK.with(|stack| {
            let mut stack = stack.borrow_mut();
            if stack.contains(&name) {
                return false;
            }
            stack.push(name);
            true
        });

        if entered {
            DependencyGuardState::Entered(Self { name })
        } else {
            DependencyGuardState::Recursive
        }
    }
}

impl Drop for SchemaDependencyGuard {
    fn drop(&mut self) {
        SCHEMA_DEPENDENCY_STACK.with(|stack| {
            let popped = stack.borrow_mut().pop();
            debug_assert_eq!(popped, Some(self.name));
        });
    }
}

/// Returns a TypeScript type reference when `T` has a stable schema name, otherwise its inline type.
///
/// This helper is public for derive macro expansion but is not part of the intended
/// high-level embedding API.
#[doc(hidden)]
pub fn schema_type_ref<T>() -> TsType
where
    T: TsSchema,
{
    match declared_schema_name(T::schema_name()) {
        Some(name) => TsType::TypeRef(name.to_owned()),
        None => T::ts_type(),
    }
}

/// The schema name of `T` when it is declared, rather than anonymous or blank.
fn declared_schema_name(name: Option<&'static str>) -> Option<&'static str> {
    name.filter(|name| !name.trim().is_empty())
}

/// Named schemas referenced by a container of `T`.
fn element_dependencies<T: TsSchema>() -> Vec<Schema> {
    let mut dependencies = Vec::new();
    push_schema_dependency::<T>(&mut dependencies);
    dependencies
}

fn array_of<T: TsSchema>() -> TsType {
    TsType::Array(Box::new(schema_type_ref::<T>()))
}

fn record_of<T: TsSchema>(key: TsRecordKey) -> TsType {
    TsType::Record {
        key,
        value: Box::new(schema_type_ref::<T>()),
    }
}

macro_rules! leaf_schemas {
    ($ts_type:expr => $($ty:ty),+ $(,)?) => {
        $(
            impl TsSchema for $ty {
                fn ts_type() -> TsType {
                    $ts_type
                }
            }
        )+
    };
}

leaf_schemas!(TsType::Void => ());
leaf_schemas!(TsType::Boolean => bool);
leaf_schemas!(TsType::String => String, char, PathBuf, IpAddr, Ipv4Addr, Ipv6Addr);
leaf_schemas!(TsType::Json => serde_json::Value);
leaf_schemas!(TsType::Number => u8, u16, u32, u64, u128, usize);
leaf_schemas!(TsType::Number => i8, i16, i32, i64, i128, isize);
leaf_schemas!(TsType::Number => f32, f64);

impl<T: TsSchema> TsSchema for Option<T> {
    fn ts_type() -> TsType {
        TsType::Nullable(Box::new(schema_type_ref::<T>()))
    }

    fn schema_dependencies() -> Vec<Schema> {
        element_dependencies::<T>()
    }
}

macro_rules! array_schemas {
    ($($container:ident),+ $(,)?) => {
        $(
            impl<T: TsSchema> TsSchema for $container<T> {
                fn ts_type() -> TsType {
                    array_of::<T>()
                }

                fn schema_dependencies() -> Vec<Schema> {
                    element_dependencies::<T>()
                }
            }
        )+
    };
}

array_schemas!(Vec, VecDeque, BTreeSet);

impl<T: TsSchema, S> TsSchema for HashSet<T, S> {
    fn ts_type() -> TsType {
        array_of::<T>()
    }

    fn schema_dependencies() -> Vec<Schema> {
        element_dependencies::<T>()
    }
}

impl<T: TsSchema> TsSchema for Box<[T]> {
    fn ts_type() -> TsType {
        array_of::<T>()
    }

    fn schema_dependencies() -> Vec<Schema> {
        element_dependencies::<T>()
    }
}

/// A tuple of exactly `N` items, since the codecs accept no other length.
impl<T: TsSchema, const N: usize> TsSchema for [T; N] {
    fn ts_type() -> TsType {
        TsType::Tuple(vec![schema_type_ref::<T>(); N])
    }

    fn schema_dependencies() -> Vec<Schema> {
        element_dependencies::<T>()
    }
}

macro_rules! transparent_schemas {
    ($($pointer:ident),+ $(,)?) => {
        $(
            impl<T: TsSchema> TsSchema for $pointer<T> {
                fn schema_name() -> Option<&'static str> {
                    T::schema_name()
                }

                fn ts_type() -> TsType {
                    T::ts_type()
                }

                fn schema_dependencies() -> Vec<Schema> {
                    T::schema_dependencies()
                }
            }
        )+
    };
}

transparent_schemas!(Box, Arc, Rc);

macro_rules! record_schemas {
    ($record_key:expr => $($key:ty),+ $(,)?) => {
        $(
            impl<T: TsSchema, S> TsSchema for HashMap<$key, T, S> {
                fn ts_type() -> TsType {
                    record_of::<T>($record_key)
                }

                fn schema_dependencies() -> Vec<Schema> {
                    element_dependencies::<T>()
                }
            }

            impl<T: TsSchema> TsSchema for BTreeMap<$key, T> {
                fn ts_type() -> TsType {
                    record_of::<T>($record_key)
                }

                fn schema_dependencies() -> Vec<Schema> {
                    element_dependencies::<T>()
                }
            }
        )+
    };
}

record_schemas!(TsRecordKey::String => String);
record_schemas!(TsRecordKey::Number => u8, u16, u32, u64, u128, usize);
record_schemas!(TsRecordKey::Number => i8, i16, i32, i64, i128, isize);

macro_rules! tuple_schema {
    ($len:literal => $($index:tt $name:ident),+) => {
        impl<$($name: TsSchema),+> TsSchema for ($($name,)+) {
            fn ts_type() -> TsType {
                TsType::Tuple(vec![$(schema_type_ref::<$name>()),+])
            }

            fn schema_dependencies() -> Vec<Schema> {
                let mut dependencies = Vec::new();
                $(push_schema_dependency::<$name>(&mut dependencies);)+
                dependencies
            }
        }
    };
}

for_each_tuple!(tuple_schema);

#[cfg(test)]
mod tests {
    use super::*;
    use crate::contract::TsField;
    use serde_json::json;

    struct NamedDependencyBomb;

    impl TsSchema for NamedDependencyBomb {
        fn schema_name() -> Option<&'static str> {
            Some("NamedDependencyBomb")
        }

        fn ts_type() -> TsType {
            TsType::Number
        }

        fn schema_dependencies() -> Vec<Schema> {
            panic!("schema_type_ref must not materialize dependencies")
        }
    }

    #[test]
    fn option_schema_is_nullable_value_not_optional_field() {
        assert_eq!(
            Option::<String>::ts_type(),
            TsType::Nullable(Box::new(TsType::String))
        );
    }

    #[test]
    fn collection_schemas_emit_arrays_records_and_tuples() {
        assert_eq!(
            Vec::<u64>::ts_type(),
            TsType::Array(Box::new(TsType::Number))
        );
        assert_eq!(
            HashSet::<String>::ts_type(),
            TsType::Array(Box::new(TsType::String))
        );
        assert_eq!(
            BTreeSet::<bool>::ts_type(),
            TsType::Array(Box::new(TsType::Boolean))
        );
        assert_eq!(<[u8; 3]>::ts_type(), TsType::Tuple(vec![TsType::Number; 3]));
        assert_eq!(
            HashMap::<String, bool>::ts_type(),
            TsType::Record {
                key: TsRecordKey::String,
                value: Box::new(TsType::Boolean),
            }
        );
        assert_eq!(
            HashMap::<u64, String>::ts_type(),
            TsType::Record {
                key: TsRecordKey::Number,
                value: Box::new(TsType::String),
            }
        );
        assert_eq!(
            BTreeMap::<i32, bool>::ts_type(),
            TsType::Record {
                key: TsRecordKey::Number,
                value: Box::new(TsType::Boolean),
            }
        );
        assert_eq!(
            <(String, u32, bool)>::ts_type(),
            TsType::Tuple(vec![TsType::String, TsType::Number, TsType::Boolean])
        );
    }

    #[test]
    fn std_string_like_schemas_emit_string() {
        assert_eq!(PathBuf::ts_type(), TsType::String);
        assert_eq!(IpAddr::ts_type(), TsType::String);
        assert_eq!(Ipv4Addr::ts_type(), TsType::String);
        assert_eq!(Ipv6Addr::ts_type(), TsType::String);
    }

    #[test]
    fn schema_type_ref_uses_name_without_materializing_dependencies() {
        assert_eq!(
            schema_type_ref::<NamedDependencyBomb>(),
            TsType::TypeRef(String::from("NamedDependencyBomb"))
        );
    }

    #[test]
    fn schema_validation_helpers_validate_json_values() {
        let schema = Schema::typed(
            "DemoInput",
            TsType::Object(vec![TsField::required("id", TsType::Number)]),
        );

        schema
            .validate_json(&json!({ "id": 1, "extra": true }))
            .expect("unknown fields are allowed by default");
        schema
            .validate_json_strict(&json!({ "id": 1 }))
            .expect("declared fields pass strict validation");

        let unknown_error = schema
            .validate_json_strict(&json!({ "id": 1, "extra": true }))
            .expect_err("strict validation rejects unknown fields");
        let type_error = schema
            .validate_json(&json!({ "id": "bad" }))
            .expect_err("type validation rejects mismatches");

        assert!(unknown_error.contains("$.extra: unknown field"));
        assert!(type_error.contains("$.id: expected number, got string"));
    }
}
