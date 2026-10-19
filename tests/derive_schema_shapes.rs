#![cfg(feature = "derive")]
// These types exist only to be described by `TsSchema`: nothing constructs them or reads their
// fields, and `default_spawn_region` is named only in a `#[serde(default)]` path that the derive
// does not expand. This is the single dead-code allowance for the schema-only shapes.
#![allow(dead_code)]

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::net::IpAddr;
use std::path::PathBuf;

use manivelle::{TsField, TsRecordKey, TsSchema, TsType};
use serde_json::json;

#[derive(TsSchema)]
#[manivelle(schema_only)]
struct InvoicePayload {
    id: u64,
    #[manivelle(rename = "displayName")]
    name: String,
    #[manivelle(optional)]
    metadata: BTreeMap<String, String>,
    flags: Vec<bool>,
}

#[derive(TsSchema)]
struct Point(f32, f32);

#[derive(TsSchema)]
struct FixedPayload {
    color: [u8; 4],
    points: [[f32; 2]; 2],
}

#[derive(TsSchema)]
struct IndexedPayload {
    users_by_id: HashMap<u64, String>,
    damage_by_source: BTreeMap<i32, f32>,
}

#[derive(TsSchema)]
struct SetPayload {
    tags: HashSet<String>,
    sorted_scores: BTreeSet<u64>,
}

type FixedHasher = BuildHasherDefault<DefaultHasher>;

#[derive(TsSchema)]
struct CustomHasherPayload {
    scores: HashMap<String, u32, FixedHasher>,
    names_by_id: HashMap<u64, String, FixedHasher>,
    tags: HashSet<String, FixedHasher>,
}

#[derive(TsSchema)]
struct HostResourcePayload {
    path: PathBuf,
    bind_addr: IpAddr,
}

#[derive(TsSchema)]
struct RecursiveNode {
    child: Option<Box<RecursiveNode>>,
}

#[derive(TsSchema)]
struct Envelope<T> {
    payload: T,
}

#[derive(TsSchema)]
#[manivelle(schema_only)]
#[serde(rename_all = "camelCase")]
struct SerdePlayerPayload {
    player_id: u64,
    critical_hits: u32,
    nickname: std::option::Option<String>,
    #[serde(default)]
    lives: u32,
    #[serde(default = "default_spawn_region")]
    spawn_region: String,
    #[serde(rename = "damage")]
    amount: f32,
    #[manivelle(rename = "manivelleName")]
    serde_priority_check: String,
    #[serde(skip)]
    internal_seed: u64,
}

fn default_spawn_region() -> String {
    String::from("arena")
}

#[derive(TsSchema)]
struct DirectionalFields {
    always: u32,
    #[serde(skip_serializing)]
    input_only: u32,
    #[serde(skip_deserializing)]
    output_only: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    maybe: Option<u32>,
}

#[derive(TsSchema)]
#[manivelle(encode_only)]
struct OutputView {
    #[serde(skip_serializing)]
    input_only: u32,
    #[serde(skip_deserializing)]
    output_only: u32,
}

#[test]
fn derive_ts_schema_for_named_struct_fields() {
    assert_eq!(InvoicePayload::schema_name(), "InvoicePayload");
    assert_eq!(
        InvoicePayload::ts_type(),
        TsType::Object(vec![
            TsField::required("id", TsType::Number),
            TsField::required("displayName", TsType::String),
            TsField::optional(
                "metadata",
                TsType::Record {
                    key: TsRecordKey::String,
                    value: Box::new(TsType::String),
                },
            ),
            TsField::required("flags", TsType::Array(Box::new(TsType::Boolean))),
        ])
    );
}

#[test]
fn derive_ts_schema_respects_serde_field_attributes() {
    assert_eq!(
        SerdePlayerPayload::ts_type(),
        TsType::Object(vec![
            TsField::required("playerId", TsType::Number),
            TsField::required("criticalHits", TsType::Number),
            TsField::optional("nickname", TsType::Nullable(Box::new(TsType::String))),
            TsField::optional("lives", TsType::Number),
            TsField::optional("spawnRegion", TsType::String),
            TsField::required("damage", TsType::Number),
            TsField::required("manivelleName", TsType::String),
        ])
    );
}

#[test]
fn derive_ts_schema_for_tuple_struct() {
    assert_eq!(
        Point::ts_type(),
        TsType::Tuple(vec![TsType::Number, TsType::Number])
    );
}

#[test]
fn derive_ts_schema_for_fixed_array_fields() {
    let pair = TsType::Tuple(vec![TsType::Number; 2]);
    assert_eq!(
        FixedPayload::ts_type(),
        TsType::Object(vec![
            TsField::required("color", TsType::Tuple(vec![TsType::Number; 4])),
            TsField::required("points", TsType::Tuple(vec![pair.clone(), pair])),
        ])
    );
    FixedPayload::validate_json(&json!({ "color": [1, 2, 3, 4], "points": [[1, 2], [3, 4]] }))
        .expect("exact lengths are valid");
    let short =
        FixedPayload::validate_json(&json!({ "color": [1, 2, 3], "points": [[1, 2], [3, 4]] }))
            .expect_err("a fixed-size array needs exactly N items");
    assert!(short.contains("$.color"), "{short}");
}

#[test]
fn derive_ts_schema_for_maps_and_sets_with_a_custom_hasher() {
    let record = |key, value| TsType::Record {
        key,
        value: Box::new(value),
    };
    assert_eq!(
        CustomHasherPayload::ts_type(),
        TsType::Object(vec![
            TsField::required("scores", record(TsRecordKey::String, TsType::Number)),
            TsField::required("names_by_id", record(TsRecordKey::Number, TsType::String)),
            TsField::required("tags", TsType::Array(Box::new(TsType::String))),
        ])
    );
}

#[test]
fn derive_ts_schema_for_numeric_record_fields() {
    assert_eq!(
        IndexedPayload::ts_type(),
        TsType::Object(vec![
            TsField::required(
                "users_by_id",
                TsType::Record {
                    key: TsRecordKey::Number,
                    value: Box::new(TsType::String),
                },
            ),
            TsField::required(
                "damage_by_source",
                TsType::Record {
                    key: TsRecordKey::Number,
                    value: Box::new(TsType::Number),
                },
            ),
        ])
    );
}

#[test]
fn derive_ts_schema_for_set_fields() {
    assert_eq!(
        SetPayload::ts_type(),
        TsType::Object(vec![
            TsField::required("tags", TsType::Array(Box::new(TsType::String))),
            TsField::required("sorted_scores", TsType::Array(Box::new(TsType::Number))),
        ])
    );
}

#[test]
fn derive_ts_schema_for_std_string_like_fields() {
    assert_eq!(
        HostResourcePayload::ts_type(),
        TsType::Object(vec![
            TsField::required("path", TsType::String),
            TsField::required("bind_addr", TsType::String),
        ])
    );
}

#[test]
fn derive_ts_schema_for_recursive_box_field() {
    assert_eq!(
        RecursiveNode::ts_type(),
        TsType::Object(vec![TsField::optional(
            "child",
            TsType::Nullable(Box::new(TsType::TypeRef(String::from("RecursiveNode")))),
        )])
    );

    let schema = RecursiveNode::schema();

    assert_eq!(schema.name, "RecursiveNode");
    assert_eq!(schema.dependencies.len(), 1);
    assert_eq!(schema.dependencies[0].name, "RecursiveNode");
}

#[test]
fn derive_ts_schema_for_generic_struct() {
    assert_eq!(
        Envelope::<String>::ts_type(),
        TsType::Object(vec![TsField::required("payload", TsType::String)])
    );
}

#[test]
fn derive_ts_schema_marks_fields_missing_in_one_direction_optional() {
    assert_eq!(
        DirectionalFields::ts_type(),
        TsType::Object(vec![
            TsField::required("always", TsType::Number),
            TsField::optional("input_only", TsType::Number),
            TsField::optional("output_only", TsType::Number),
            TsField::optional("maybe", TsType::Nullable(Box::new(TsType::Number))),
        ])
    );
    assert_eq!(
        OutputView::ts_type(),
        TsType::Object(vec![TsField::required("output_only", TsType::Number)])
    );
}
