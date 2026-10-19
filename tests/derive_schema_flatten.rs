#![cfg(feature = "derive")]

mod support;

use std::collections::{BTreeMap, HashMap};

use manivelle::{
    Engine, EngineError, EngineOptions, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, InMemoryHostContractRegistry, TsEnumVariant, TsField, TsSchema, TsType,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

#[derive(TsSchema)]
#[serde(rename_all = "camelCase")]
struct ActorIdentity {
    actor_id: u64,
    display_name: String,
}

#[derive(TsSchema)]
#[serde(rename_all = "camelCase")]
struct FlattenedActionPayload {
    action_id: String,
    #[serde(flatten)]
    actor: ActorIdentity,
    confirmed: bool,
}

#[derive(TsSchema)]
struct Revision(u32);

#[derive(TsSchema)]
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

fn scores_fields() -> Vec<TsField> {
    vec![
        TsField::required("playerName", TsType::String),
        TsField::optional("bonus", TsType::Nullable(Box::new(TsType::Boolean))),
    ]
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
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(OpenEvent::Scores(input))
    }
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
    let mut engine = Engine::new(&EngineOptions::default()).expect("create engine");
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
