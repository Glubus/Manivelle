#![cfg(feature = "derive")]

use manivelle::{TsEnumVariant, TsField, TsLiteral, TsSchema, TsType};

#[derive(TsSchema)]
#[manivelle(name = "BillingStatus")]
enum Status {
    Pending,
    Paid,
}

#[derive(TsSchema)]
#[serde(rename_all = "kebab-case")]
enum SerdeMode {
    FastMode,
    #[serde(rename = "manual-mode")]
    SlowMode,
}

#[derive(TsSchema)]
enum HostMessage<T> {
    Ready,
    User { id: u64, name: String },
    Data(T),
    Pair(String, bool),
}

#[derive(TsSchema)]
#[serde(untagged)]
enum LookupInput {
    Id(u64),
    Query {
        #[serde(rename = "accountId")]
        account_id: String,
        include_disabled: bool,
    },
    Tags(Vec<String>),
}

#[derive(TsSchema)]
#[serde(tag = "kind")]
enum InternallyTaggedHostEvent {
    Connected { user_id: u64 },
    Disconnected,
}

#[derive(TsSchema)]
#[serde(tag = "kind", content = "payload")]
enum AdjacentlyTaggedHostEvent {
    Connected { user_id: u64 },
    Tags(Vec<String>),
    Disconnected,
}

#[derive(TsSchema)]
#[serde(tag = "kind")]
enum Signal {
    Start,
    Stop,
}

#[test]
fn derive_ts_schema_respects_serde_enum_variant_attributes() {
    assert_eq!(
        SerdeMode::ts_type(),
        TsType::Enum {
            tag: None,
            variants: vec![
                TsEnumVariant::unit("fast-mode"),
                TsEnumVariant::unit("manual-mode"),
            ],
        }
    );
}

#[test]
fn derive_ts_schema_for_unit_enum() {
    assert_eq!(Status::schema_name(), "BillingStatus");
    assert!(matches!(Status::ts_type(), TsType::Enum { .. }));
}

#[test]
fn derive_ts_schema_for_payload_enum() {
    assert_eq!(
        HostMessage::<String>::ts_type(),
        TsType::Union(vec![
            TsType::Literal(TsLiteral::String(String::from("Ready"))),
            TsType::Object(vec![TsField::required(
                "User",
                TsType::Object(vec![
                    TsField::required("id", TsType::Number),
                    TsField::required("name", TsType::String),
                ]),
            )]),
            TsType::Object(vec![TsField::required("Data", TsType::String)]),
            TsType::Object(vec![TsField::required(
                "Pair",
                TsType::Tuple(vec![TsType::String, TsType::Boolean]),
            )]),
        ])
    );
}

#[test]
fn derive_ts_schema_for_serde_untagged_enum() {
    assert_eq!(
        LookupInput::ts_type(),
        TsType::Union(vec![
            TsType::Number,
            TsType::Object(vec![
                TsField::required("accountId", TsType::String),
                TsField::required("include_disabled", TsType::Boolean),
            ]),
            TsType::Array(Box::new(TsType::String)),
        ])
    );
}

#[test]
fn derive_ts_schema_respects_serde_tagged_enum_attributes() {
    assert_eq!(
        InternallyTaggedHostEvent::ts_type(),
        TsType::Enum {
            tag: Some(String::from("kind")),
            variants: vec![
                TsEnumVariant::payload(
                    "Connected",
                    vec![TsField::required("user_id", TsType::Number)],
                ),
                TsEnumVariant::unit("Disconnected"),
            ],
        }
    );
}

#[test]
fn derive_ts_schema_respects_serde_adjacently_tagged_enum_attributes() {
    assert_eq!(
        AdjacentlyTaggedHostEvent::ts_type(),
        TsType::Enum {
            tag: Some(String::from("kind")),
            variants: vec![
                TsEnumVariant::payload(
                    "Connected",
                    vec![TsField::required(
                        "payload",
                        TsType::Object(vec![TsField::required("user_id", TsType::Number)]),
                    )],
                ),
                TsEnumVariant::payload(
                    "Tags",
                    vec![TsField::required(
                        "payload",
                        TsType::Array(Box::new(TsType::String)),
                    )],
                ),
                TsEnumVariant::unit("Disconnected"),
            ],
        }
    );
}

#[test]
fn derive_ts_schema_keeps_tag_objects_for_unit_only_tagged_enums() {
    let tagged = |name: &str| {
        TsType::Object(vec![TsField::required(
            "kind",
            TsType::Literal(TsLiteral::String(String::from(name))),
        )])
    };
    assert_eq!(
        Signal::ts_type(),
        TsType::Union(vec![tagged("Start"), tagged("Stop")])
    );
}
