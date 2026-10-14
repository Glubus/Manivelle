#![cfg(feature = "derive")]

#[allow(dead_code)] // shared helpers: this suite only needs a temp dir and tsc
mod support;

use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::hash::{BuildHasherDefault, DefaultHasher};
use std::net::IpAddr;
use std::path::PathBuf;

use manivelle::{
    Engine, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
    InMemoryHostContractRegistry, TsEnumVariant, TsField, TsLiteral, TsRecordKey, TsSchema, TsType,
    VmError, VmOptions,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(TsSchema)]
#[manivelle(schema_only)]
#[allow(dead_code)]
struct InvoicePayload {
    id: u64,
    #[manivelle(rename = "displayName")]
    name: String,
    #[manivelle(optional)]
    metadata: BTreeMap<String, String>,
    flags: Vec<bool>,
}

#[derive(TsSchema)]
#[manivelle(name = "BillingStatus")]
#[allow(dead_code)]
enum Status {
    Pending,
    Paid,
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct Point(f32, f32);

#[derive(Debug, Deserialize, TsSchema)]
#[serde(transparent)]
#[allow(dead_code)]
struct UserId(u64);

#[derive(Debug, Serialize, TsSchema)]
#[serde(transparent)]
#[allow(dead_code)]
struct SessionToken {
    value: String,
}

struct FindSession;
struct AutoCreateInvoice;
struct AutoInvoiceCreated;

#[derive(Debug, Deserialize, TsSchema)]
#[allow(dead_code)]
struct FindSessionInput {
    user_id: UserId,
}

#[derive(Debug, Serialize, TsSchema)]
#[allow(dead_code)]
struct FindSessionOutput {
    token: SessionToken,
}

#[derive(Debug, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AutoCreateInvoiceInput {
    account_id: String,
    total: f32,
}

#[derive(Debug, Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AutoCreateInvoiceOutput {
    invoice_id: String,
    accepted: bool,
}

#[derive(Debug, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct AutoInvoiceCreatedPayload {
    invoice_id: String,
    total: f32,
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct FixedPayload {
    color: [u8; 4],
    points: [[f32; 2]; 2],
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct IndexedPayload {
    users_by_id: HashMap<u64, String>,
    damage_by_source: BTreeMap<i32, f32>,
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct SetPayload {
    tags: HashSet<String>,
    sorted_scores: BTreeSet<u64>,
}

type FixedHasher = BuildHasherDefault<DefaultHasher>;

#[derive(TsSchema)]
#[allow(dead_code)]
struct CustomHasherPayload {
    scores: HashMap<String, u32, FixedHasher>,
    names_by_id: HashMap<u64, String, FixedHasher>,
    tags: HashSet<String, FixedHasher>,
}

#[derive(Debug, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "kebab-case")]
struct KebabSettings {
    max_speed: f64,
    #[serde(rename = "default")]
    fallback: bool,
    #[serde(rename = "2d")]
    flat: bool,
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct HostResourcePayload {
    path: PathBuf,
    bind_addr: IpAddr,
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct RecursiveNode {
    child: Option<Box<RecursiveNode>>,
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct Envelope<T> {
    payload: T,
}

#[derive(TsSchema)]
#[allow(dead_code)]
enum HostMessage<T> {
    Ready,
    User { id: u64, name: String },
    Data(T),
    Pair(String, bool),
}

#[derive(TsSchema)]
#[serde(untagged)]
#[allow(dead_code)]
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
#[allow(dead_code)]
enum InternallyTaggedHostEvent {
    Connected { user_id: u64 },
    Disconnected,
}

#[derive(TsSchema)]
#[serde(tag = "kind", content = "payload")]
#[allow(dead_code)]
enum AdjacentlyTaggedHostEvent {
    Connected { user_id: u64 },
    Tags(Vec<String>),
    Disconnected,
}

struct CreateInvoice;

#[derive(Debug, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct CreateInvoiceInput {
    account_id: String,
    total: f32,
    memo: Option<String>,
    #[serde(default)]
    retry_count: u32,
}

#[derive(Debug, Serialize, TsSchema)]
#[manivelle(name = "InvoiceCreated")]
#[allow(dead_code)]
struct CreateInvoiceOutput {
    invoice_id: String,
    accepted: bool,
}

impl HostContract for CreateInvoice {
    const NAME: &'static str = "billing.invoice.create";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for CreateInvoice {
    type Input = CreateInvoiceInput;
    type Output = CreateInvoiceOutput;
}

impl HostFunction for CreateInvoice {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(CreateInvoiceOutput {
            invoice_id: format!("invoice:{}", input.account_id),
            accepted: input.total > 0.0,
        })
    }
}

impl HostContract for FindSession {
    const NAME: &'static str = "session.find";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindSession {
    type Input = FindSessionInput;
    type Output = FindSessionOutput;
}

impl HostFunction for FindSession {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(FindSessionOutput {
            token: SessionToken {
                value: format!("session:{}", input.user_id.0),
            },
        })
    }
}

impl HostContract for AutoCreateInvoice {
    const NAME: &'static str = "billing.invoice.autoCreate";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for AutoCreateInvoice {
    type Input = AutoCreateInvoiceInput;
    type Output = AutoCreateInvoiceOutput;
}

impl HostFunction for AutoCreateInvoice {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(AutoCreateInvoiceOutput {
            invoice_id: format!("invoice:{}", input.account_id),
            accepted: input.total > 0.0,
        })
    }
}

impl HostContract for AutoInvoiceCreated {
    const NAME: &'static str = "billing.invoice.created";

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl manivelle::HostCallback for AutoInvoiceCreated {
    type Payload = AutoInvoiceCreatedPayload;
}

#[derive(TsSchema)]
#[manivelle(schema_only)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
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

#[derive(TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ActorIdentity {
    actor_id: u64,
    display_name: String,
}

#[derive(TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct FlattenedActionPayload {
    action_id: String,
    #[serde(flatten)]
    actor: ActorIdentity,
    confirmed: bool,
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct Revision(u32);

#[derive(TsSchema)]
#[allow(dead_code)]
struct InvalidNewtypeFlatten {
    #[serde(flatten)]
    revision: Revision,
}

/// Serde's open-object shape: declared fields, then every other key in the map.
#[derive(Debug, Serialize, Deserialize, TsSchema)]
struct Open {
    name: String,
    #[serde(flatten)]
    rest: BTreeMap<String, Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct OpenScores {
    player_name: String,
    bonus: Option<bool>,
    #[serde(flatten)]
    scores: HashMap<String, u32>,
}

#[derive(Debug, Serialize, Deserialize, TsSchema)]
struct OpenLabels {
    id: u32,
    #[serde(flatten)]
    labels: Option<BTreeMap<String, String>>,
}

#[derive(Debug, Serialize, Deserialize, TsSchema)]
struct NestedOpenScores {
    round: u32,
    #[serde(flatten)]
    scores: OpenScores,
}

#[derive(Debug, Serialize, Deserialize, TsSchema)]
#[serde(tag = "kind")]
enum OpenEvent {
    Custom {
        id: u32,
        #[serde(flatten)]
        data: BTreeMap<String, u32>,
    },
    Scores(OpenScores),
    Closed,
}

#[allow(dead_code)]
fn default_spawn_region() -> String {
    String::from("arena")
}

#[derive(TsSchema)]
#[serde(rename_all = "kebab-case")]
#[allow(dead_code)]
enum SerdeMode {
    FastMode,
    #[serde(rename = "manual-mode")]
    SlowMode,
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
fn derive_ts_schema_flattens_object_fields() {
    assert_eq!(
        FlattenedActionPayload::ts_type(),
        TsType::Object(vec![
            TsField::required("actionId", TsType::String),
            TsField::required("actorId", TsType::Number),
            TsField::required("displayName", TsType::String),
            TsField::required("confirmed", TsType::Boolean),
        ])
    );
}

#[test]
#[should_panic(expected = "serde flatten requires a TsSchema object or map type")]
fn derive_ts_schema_rejects_non_object_flatten_schema() {
    let _ = InvalidNewtypeFlatten::ts_type();
}

fn scores_fields() -> Vec<TsField> {
    vec![
        TsField::required("playerName", TsType::String),
        TsField::optional("bonus", TsType::Nullable(Box::new(TsType::Boolean))),
    ]
}

#[test]
fn derive_ts_schema_opens_objects_for_flattened_maps() {
    assert_eq!(
        Open::ts_type(),
        TsType::OpenObject {
            fields: vec![TsField::required("name", TsType::String)],
            rest: Box::new(TsType::Json),
        }
    );
    assert_eq!(
        OpenScores::ts_type(),
        TsType::OpenObject {
            fields: scores_fields(),
            rest: Box::new(TsType::Number),
        }
    );
    assert_eq!(
        OpenLabels::ts_type(),
        TsType::OpenObject {
            fields: vec![TsField::required("id", TsType::Number)],
            rest: Box::new(TsType::String),
        }
    );
    let mut nested_fields = vec![TsField::required("round", TsType::Number)];
    nested_fields.extend(scores_fields());
    assert_eq!(
        NestedOpenScores::ts_type(),
        TsType::OpenObject {
            fields: nested_fields,
            rest: Box::new(TsType::Number),
        }
    );
}

#[test]
fn derive_ts_schema_opens_internally_tagged_variants_with_flattened_maps() {
    assert_eq!(
        OpenEvent::ts_type(),
        TsType::Enum {
            tag: Some(String::from("kind")),
            variants: vec![
                TsEnumVariant::payload("Custom", vec![TsField::required("id", TsType::Number)])
                    .with_rest(Some(TsType::Number)),
                TsEnumVariant::payload("Scores", scores_fields()).with_rest(Some(TsType::Number)),
                TsEnumVariant::unit("Closed"),
            ],
        }
    );
}

fn sample_scores() -> OpenScores {
    OpenScores {
        player_name: String::from("ada"),
        bonus: None,
        scores: HashMap::from([(String::from("round1"), 3), (String::from("round2"), 5)]),
    }
}

fn assert_validates_serde_output<T: TsSchema + Serialize>(value: &T) {
    let wire = serde_json::to_value(value).expect("serialize with serde");
    T::validate_json(&wire).unwrap_or_else(|error| panic!("rejects {wire}: {error}"));
    T::validate_json_strict(&wire).unwrap_or_else(|error| panic!("strict rejects {wire}: {error}"));
}

fn assert_rejected<T: TsSchema>(value: &Value, expected: &str) {
    for error in [
        T::validate_json(value).expect_err("validation rejects"),
        T::validate_json_strict(value).expect_err("strict validation rejects"),
    ] {
        assert!(error.contains(expected), "{error:?} lacks {expected:?}");
    }
}

#[test]
fn flattened_map_schemas_validate_serde_output() {
    assert_validates_serde_output(&Open {
        name: String::from("a"),
        rest: BTreeMap::from([
            (String::from("x"), json!(1)),
            (String::from("y"), json!([true, null])),
        ]),
    });
    assert_validates_serde_output(&sample_scores());
    assert_validates_serde_output(&OpenLabels {
        id: 1,
        labels: Some(BTreeMap::from([(
            String::from("env"),
            String::from("prod"),
        )])),
    });
    assert_validates_serde_output(&OpenLabels {
        id: 2,
        labels: None,
    });
    assert_validates_serde_output(&NestedOpenScores {
        round: 2,
        scores: sample_scores(),
    });
    assert_validates_serde_output(&OpenEvent::Custom {
        id: 7,
        data: BTreeMap::from([(String::from("clicks"), 2)]),
    });
    assert_validates_serde_output(&OpenEvent::Scores(sample_scores()));
    assert_validates_serde_output(&OpenEvent::Closed);
}

#[test]
fn flattened_map_schemas_reject_wrong_values() {
    assert_rejected::<OpenScores>(
        &json!({ "playerName": "ada", "round1": "high" }),
        "$.round1: expected number, got string",
    );
    assert_rejected::<OpenScores>(
        &json!({ "playerName": 3, "round1": 1 }),
        "$.playerName: expected string, got number",
    );
    assert_rejected::<NestedOpenScores>(
        &json!({ "round": 1, "playerName": "ada", "extra": false }),
        "$.extra: expected number, got boolean",
    );
    assert_rejected::<OpenEvent>(
        &json!({ "kind": "Custom", "id": 1, "clicks": "two" }),
        "$.clicks: expected number, got string",
    );
    let closed_error = OpenEvent::validate_json_strict(&json!({ "kind": "Closed", "extra": 1 }))
        .expect_err("strict validation keeps variants without a map closed");
    assert!(closed_error.contains("$.extra: unknown field"));
}

#[test]
fn flattened_map_sdk_predicates_check_every_key() {
    let mut engine = Engine::new(&VmOptions::default()).expect("create engine");
    engine
        .registry()
        .function::<RecordScores>()
        .expect("register flattened-map host function");
    let sdk = engine.registry().sdk().expect("render SDK");
    let source = format!(
        "{sdk}\nexport function isScores(value: unknown): boolean {{ return models.OpenScores.is(value); }}\n"
    );
    engine.load_script("predicates", &source).expect("load SDK");
    let is_scores = |value: Value| -> bool {
        engine
            .call("predicates", "isScores", (value,))
            .expect("call predicate")
    };

    assert!(is_scores(
        serde_json::to_value(sample_scores()).expect("serialize")
    ));
    assert!(!is_scores(json!({ "playerName": "ada", "round1": "high" })));
    assert!(!is_scores(json!({ "playerName": "ada", "bonus": 1 })));
    assert!(!is_scores(json!({ "round1": 1 })));
}

struct RecordScores;

impl HostContract for RecordScores {
    const NAME: &'static str = "scores.record";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for RecordScores {
    type Input = OpenScores;
    type Output = OpenEvent;
}

impl HostFunction for RecordScores {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(OpenEvent::Scores(input))
    }
}

#[test]
fn flattened_map_declarations_typecheck() {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .function::<RecordScores>()
        .expect("register flattened-map host function");
    let dts = registry.dts().expect("render declarations");
    let sdk = registry.sdk().expect("render SDK");

    let scores = "type OpenScores = { playerName: string; bonus?: boolean | null; [key: string]: number | string | boolean | null | undefined; };";
    assert!(dts.contains(scores), "{dts}");
    assert!(sdk.contains(scores), "{sdk}");
    assert!(dts.contains("{ kind: \"Custom\"; id: number; [key: string]: number | \"Custom\"; }"));

    let cache_dir = support::TestCacheDir::new("derive-schema-flatten-tsc");
    let sdk_path = cache_dir.path().join("sdk.ts");
    std::fs::write(&sdk_path, flattened_sdk_usage(&sdk)).expect("write sdk");
    let Some(output) = support::run_tsc(&sdk_path) else {
        return;
    };
    assert!(
        output.status.success(),
        "generated sdk failed tsc\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn flattened_sdk_usage(sdk: &str) -> String {
    format!(
        "{sdk}\n\
const sample: OpenScores = {{ playerName: \"ada\", bonus: null, round1: 3 }};\n\
const custom: OpenEvent = {{ kind: \"Custom\", id: 1, clicks: 2 }};\n\
const nested: OpenEvent = {{ kind: \"Scores\", playerName: \"ada\", round2: 5 }};\n\
const closed: OpenEvent = {{ kind: \"Closed\" }};\n\
// @ts-expect-error extra values must match the flattened map\n\
const wrong: OpenScores = {{ playerName: \"ada\", round1: [1] }};\n\
if (models.OpenScores.is(sample)) {{\n\
  models.OpenScores.create(sample).playerName.toUpperCase();\n\
}}\n\
const recorded: OpenEvent = call(\"scores.record\", sample);\n\
export {{ custom, nested, closed, wrong, recorded }};\n"
    )
}

#[test]
fn derived_schema_validates_json_values_with_ergonomic_hooks() {
    let valid = json!({
        "actionId": "act-1",
        "actorId": 7,
        "displayName": "Nami",
        "confirmed": true,
    });
    let unknown_field = json!({
        "actionId": "act-1",
        "actorId": 7,
        "displayName": "Nami",
        "confirmed": true,
        "actor": { "actorId": 7, "displayName": "Nami" },
    });
    let invalid = json!({
        "actionId": "act-1",
        "actorId": "bad",
        "displayName": "Nami",
        "confirmed": true,
    });

    FlattenedActionPayload::validate_json(&valid).expect("valid flattened value");
    FlattenedActionPayload::validate_json(&unknown_field).expect("unknown fields allowed");
    FlattenedActionPayload::validate_json_strict(&valid).expect("valid strict flattened value");

    let unknown_error = FlattenedActionPayload::validate_json_strict(&unknown_field)
        .expect_err("strict validation rejects unknown fields");
    let invalid_error =
        FlattenedActionPayload::validate_json(&invalid).expect_err("invalid nested type");

    assert!(unknown_error.contains("$.actor: unknown field"));
    assert!(invalid_error.contains("$.actorId: expected number, got string"));
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
fn derive_ts_schema_for_tuple_struct() {
    assert_eq!(
        Point::ts_type(),
        TsType::Tuple(vec![TsType::Number, TsType::Number])
    );
}

#[test]
fn derive_ts_schema_for_serde_transparent_newtypes() {
    assert_eq!(UserId::schema_name(), "UserId");
    assert_eq!(UserId::ts_type(), TsType::Number);
    assert_eq!(SessionToken::schema_name(), "SessionToken");
    assert_eq!(SessionToken::ts_type(), TsType::String);
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

struct ApplySettings;

impl HostContract for ApplySettings {
    const NAME: &'static str = "settings.apply";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for ApplySettings {
    type Input = KebabSettings;
    type Output = KebabSettings;
}

impl HostFunction for ApplySettings {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(input)
    }
}

#[test]
fn renamed_fields_that_are_not_identifiers_are_quoted_in_declarations() {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .function::<ApplySettings>()
        .expect("register kebab-case host function");
    let dts = registry.dts().expect("render declarations");
    let sdk = registry.sdk().expect("render SDK");

    let settings =
        r#"type KebabSettings = { "max-speed": number; default: boolean; "2d": boolean; };"#;
    assert!(dts.contains(settings), "{dts}");
    assert!(sdk.contains(settings), "{sdk}");

    let cache_dir = support::TestCacheDir::new("derive-schema-quoted-keys-tsc");
    let sdk_path = cache_dir.path().join("sdk.ts");
    let usage = format!(
        "{sdk}\n\
const sample: KebabSettings = {{ \"max-speed\": 1.5, default: true, \"2d\": false }};\n\
// @ts-expect-error a quoted field keeps its declared type\n\
const wrong: KebabSettings = {{ \"max-speed\": \"fast\", default: true, \"2d\": false }};\n\
if (models.KebabSettings.is(sample)) {{\n\
  models.KebabSettings.create(sample)[\"max-speed\"].toFixed();\n\
}}\n\
const applied: KebabSettings = call(\"settings.apply\", sample);\n\
export {{ wrong, applied }};\n"
    );
    std::fs::write(&sdk_path, usage).expect("write sdk");
    let Some(output) = support::run_tsc(&sdk_path) else {
        return;
    };
    assert!(
        output.status.success(),
        "generated sdk failed tsc\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
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

#[derive(TsSchema)]
#[serde(tag = "kind")]
#[allow(dead_code)]
enum Signal {
    Start,
    Stop,
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

#[derive(TsSchema)]
#[allow(dead_code)]
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
#[allow(dead_code)]
struct OutputView {
    #[serde(skip_serializing)]
    input_only: u32,
    #[serde(skip_deserializing)]
    output_only: u32,
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

#[test]
fn derived_schema_can_drive_host_contract_dts() {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .function::<CreateInvoice>()
        .expect("register derived-schema host function");

    let dts = registry.dts().expect("render derived-schema declarations");

    assert!(dts.contains(
        "type CreateInvoiceInput = { accountId: string; total: number; memo?: string | null; retryCount?: number; };"
    ));
    assert!(dts.contains("type InvoiceCreated = { invoice_id: string; accepted: boolean; };"));
    assert!(dts.contains(
        "declare namespace billing {\n  namespace invoice {\n    export function create(input: CreateInvoiceInput): InvoiceCreated;\n  }\n}"
    ));
}

#[test]
fn transparent_newtypes_drive_host_contract_dts() {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .function::<FindSession>()
        .expect("register transparent-newtype host function");

    let dts = registry
        .dts()
        .expect("render transparent-newtype declarations");

    assert!(dts.contains("type UserId = number;"));
    assert!(dts.contains("type SessionToken = string;"));
    assert!(dts.contains("type FindSessionInput = { user_id: UserId; };"));
    assert!(dts.contains("type FindSessionOutput = { token: SessionToken; };"));
    assert!(dts.contains(
        "declare namespace session {\n  export function find(input: FindSessionInput): FindSessionOutput;\n}"
    ));

    let sdk = registry
        .sdk()
        .expect("render transparent-newtype SDK declarations");

    assert!(sdk.contains("type UserId = number;"));
    assert!(sdk.contains("type SessionToken = string;"));
    assert!(sdk.contains("type FindSessionInput = { user_id: UserId; };"));
    assert!(sdk.contains("type FindSessionOutput = { token: SessionToken; };"));
}

#[test]
fn typed_function_registration_uses_input_and_output_ts_schema() {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .function::<AutoCreateInvoice>()
        .expect("register typed derived-schema host function");

    let descriptor = registry
        .descriptor(AutoCreateInvoice::NAME)
        .expect("get descriptor")
        .expect("typed function descriptor");
    let function = descriptor.function.expect("function metadata");
    let dts = registry.dts().expect("render declarations");
    let sdk = registry.sdk().expect("render SDK");

    assert_eq!(descriptor.schema.name, "AutoCreateInvoiceInput");
    assert_eq!(function.input_schema.name, "AutoCreateInvoiceInput");
    assert_eq!(function.output_schema.name, "AutoCreateInvoiceOutput");
    assert!(dts.contains("type AutoCreateInvoiceInput = { accountId: string; total: number; };"));
    assert!(
        dts.contains("type AutoCreateInvoiceOutput = { invoiceId: string; accepted: boolean; };")
    );
    assert!(dts.contains(
        "declare namespace billing {\n  namespace invoice {\n    export function autoCreate(input: AutoCreateInvoiceInput): AutoCreateInvoiceOutput;\n  }\n}"
    ));
    assert!(sdk.contains("autoCreate(input: AutoCreateInvoiceInput): AutoCreateInvoiceOutput"));
}

#[test]
fn typed_callback_registration_uses_payload_ts_schema() {
    let registry = InMemoryHostContractRegistry::new();
    registry
        .callback::<AutoInvoiceCreated>()
        .expect("register typed derived-schema callback");

    let descriptor = registry
        .descriptor(AutoInvoiceCreated::NAME)
        .expect("get descriptor")
        .expect("typed callback descriptor");
    let callback = descriptor.callback.expect("callback metadata");
    let dts = registry.dts().expect("render declarations");
    let sdk = registry.sdk().expect("render SDK");

    assert_eq!(descriptor.schema.name, "AutoInvoiceCreatedPayload");
    assert_eq!(callback.payload_schema.name, "AutoInvoiceCreatedPayload");
    assert!(
        dts.contains("type AutoInvoiceCreatedPayload = { invoiceId: string; total: number; };")
    );
    assert!(dts.contains("\"billing.invoice.created\": AutoInvoiceCreatedPayload;"));
    assert!(sdk.contains("created(handler: HostEventHandler<\"billing.invoice.created\">): void"));
}
