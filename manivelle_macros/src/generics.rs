use syn::{Generics, TypeParamBound, WherePredicate};

/// Adds `bound` to every type parameter, like `impl<T: bound> Trait for Type<T>`.
pub(crate) fn bound_type_params(generics: &Generics, bound: &TypeParamBound) -> Generics {
    let mut generics = generics.clone();
    for param in generics.type_params_mut() {
        param.bounds.push(bound.clone());
    }
    generics
}

/// Adds one where-clause predicate, such as `Self: Serialize` for JSON-codec impls.
pub(crate) fn with_predicate(generics: &Generics, predicate: WherePredicate) -> Generics {
    let mut generics = generics.clone();
    generics.make_where_clause().predicates.push(predicate);
    generics
}
