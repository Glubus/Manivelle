use crate::contract::{TsEnumVariant, TsField, TsLiteral, TsRecordKey, TsType};

pub(super) fn render_value_predicate(ty: &TsType, expression: &str) -> String {
    match ty {
        TsType::Unknown | TsType::Json | TsType::TypeRef(_) => String::from("true"),
        TsType::Void => format!("{expression} === undefined"),
        TsType::Boolean => format!("typeof {expression} === \"boolean\""),
        TsType::Number => {
            format!("typeof {expression} === \"number\" && Number.isFinite({expression})")
        }
        TsType::String => format!("typeof {expression} === \"string\""),
        TsType::Uint8Array => format!("{expression} instanceof Uint8Array"),
        TsType::Null => format!("{expression} === null"),
        TsType::Literal(literal) => render_literal_predicate(literal, expression),
        TsType::Object(fields) => render_object_predicate(fields, None, expression),
        TsType::OpenObject { fields, rest } => {
            render_object_predicate(fields, Some(rest.as_ref()), expression)
        }
        TsType::Array(item) => {
            let item_predicate = render_value_predicate(item, "item");
            format!("__isArrayOf({expression}, item => {item_predicate})")
        }
        TsType::Tuple(items) => render_tuple_predicate(items, expression),
        TsType::Enum { tag, variants } => {
            render_enum_predicate(tag.as_deref(), variants, expression)
        }
        TsType::Record { key, value } => render_record_predicate(*key, value, expression),
        TsType::Union(types) => any_of(
            types
                .iter()
                .map(|ty| render_value_predicate(ty, expression))
                .collect(),
        ),
        TsType::Optional(inner) => any_of(vec![
            format!("{expression} === undefined"),
            render_value_predicate(inner, expression),
        ]),
        TsType::Nullable(inner) => any_of(vec![
            format!("{expression} === null"),
            render_value_predicate(inner, expression),
        ]),
    }
}

fn render_literal_predicate(literal: &TsLiteral, expression: &str) -> String {
    match literal {
        TsLiteral::String(value) => format!("{expression} === {value:?}"),
        TsLiteral::Number(value) => format!("{expression} === {value}"),
        TsLiteral::Boolean(value) => format!("{expression} === {value}"),
    }
}

fn render_object_predicate(fields: &[TsField], rest: Option<&TsType>, expression: &str) -> String {
    let base = format!("__isRecord({expression})");
    let field_predicates = fields
        .iter()
        .map(|field| render_field_predicate(field, expression));
    let rest_predicate = rest.map(|rest| {
        let declared = fields.iter().map(|field| field.name.as_str());
        render_rest_predicate(declared, rest, expression)
    });
    join_predicates(
        std::iter::once(base)
            .chain(field_predicates)
            .chain(rest_predicate)
            .collect(),
        " && ",
    )
}

/// Checks every key outside `declared` against the `rest` type of an open object.
fn render_rest_predicate<'a>(
    declared: impl Iterator<Item = &'a str>,
    rest: &TsType,
    expression: &str,
) -> String {
    let value_predicate = render_value_predicate(rest, "item");
    let key_checks = declared.map(|name| format!("key === {name:?}"));
    let predicate = any_of(key_checks.chain([value_predicate]).collect());
    format!("__isRecordOf({expression}, (item, key) => {predicate})")
}

fn render_field_predicate(field: &TsField, expression: &str) -> String {
    let access = format!("__field({expression}, {:?})", field.name);
    let predicate = render_value_predicate(&field.ty, &access);
    if field.optional {
        any_of(vec![format!("{access} === undefined"), predicate])
    } else {
        predicate
    }
}

fn render_tuple_predicate(items: &[TsType], expression: &str) -> String {
    let base = format!(
        "Array.isArray({expression}) && {expression}.length === {}",
        items.len()
    );
    let item_predicates = items
        .iter()
        .enumerate()
        .map(|(index, ty)| {
            render_value_predicate(ty, &format!("({expression} as unknown[])[{index}]"))
        })
        .collect::<Vec<_>>();
    join_predicates(
        std::iter::once(base).chain(item_predicates).collect(),
        " && ",
    )
}

fn render_enum_predicate(
    tag: Option<&str>,
    variants: &[TsEnumVariant],
    expression: &str,
) -> String {
    if variants.iter().all(TsEnumVariant::is_unit) {
        let predicates = variants
            .iter()
            .map(|variant| format!("{expression} === {:?}", variant.name))
            .collect::<Vec<_>>();
        return any_of(predicates);
    }

    let tag = tag.unwrap_or("type");
    let variant_predicates = variants
        .iter()
        .map(|variant| {
            let tag_access = format!("__field({expression}, {tag:?})");
            let mut predicates = vec![format!("{tag_access} === {:?}", variant.name)];
            predicates.extend(
                variant
                    .fields
                    .iter()
                    .map(|field| render_field_predicate(field, expression)),
            );
            if let Some(rest) = &variant.rest {
                let declared = std::iter::once(tag)
                    .chain(variant.fields.iter().map(|field| field.name.as_str()));
                predicates.push(render_rest_predicate(declared, rest, expression));
            }
            format!("({})", join_predicates(predicates, " && "))
        })
        .collect::<Vec<_>>();
    format!(
        "{} && ({})",
        render_object_predicate(&[], None, expression),
        join_predicates(variant_predicates, " || ")
    )
}

fn render_record_predicate(key: TsRecordKey, value: &TsType, expression: &str) -> String {
    let value_predicate = render_value_predicate(value, "item");
    let key_predicate = match key {
        TsRecordKey::String => String::from("true"),
        TsRecordKey::Number => String::from("Number.isFinite(Number(key))"),
    };
    format!("__isRecordOf({expression}, (item, key) => {key_predicate} && ({value_predicate}))")
}

fn join_predicates(predicates: Vec<String>, separator: &str) -> String {
    if predicates.is_empty() {
        return String::from("true");
    }
    predicates.join(separator)
}

/// Disjunction wrapped in parentheses, so it stays one operand inside `&&` chains.
fn any_of(predicates: Vec<String>) -> String {
    format!("({})", join_predicates(predicates, " || "))
}
