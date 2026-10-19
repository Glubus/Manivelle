#![cfg(feature = "derive")]

use manivelle::{
    ContractValidation, Engine, EngineError, HostCallback, HostContext, HostContract,
    HostContractKind, HostFunction, HostFunctionSignature, HostMetadata, Schema, TsField, TsSchema,
    TsType, UnknownFieldValidation,
};
use serde_json::{Value, json};
use std::process::Command;

mod support;

use support::TestCacheDir;

struct FindUser;
struct ScoreUpdate;
struct OverlayContext;
struct RecordAction;

#[derive(TsSchema)]
#[serde(transparent)]
#[allow(dead_code)]
struct FindUserInput(u64);

#[derive(TsSchema)]
#[serde(transparent)]
#[allow(dead_code)]
struct FindUserOutput(String);

impl HostContract for FindUser {
    const NAME: &'static str = "user.find";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["user", "find"];

    fn metadata() -> HostMetadata {
        HostMetadata {
            name: String::from(Self::NAME),
            tags: vec![String::from("sdk")],
        }
    }

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindUser {
    type Input = FindUserInput;
    type Output = FindUserOutput;
}

impl HostFunction for FindUser {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(FindUserOutput(format!("user-{}", input.0)))
    }
}

#[derive(TsSchema)]
#[allow(dead_code)]
struct ScoreUpdatePayload {
    combo: u32,
}

impl HostContract for ScoreUpdate {
    const NAME: &'static str = "score.update";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["score", "onUpdate"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for ScoreUpdate {
    type Payload = ScoreUpdatePayload;
}

impl HostContract for OverlayContext {
    const NAME: &'static str = "overlay";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["overlay"];

    fn kind() -> HostContractKind {
        HostContractKind::Context
    }
}

impl HostContext for OverlayContext {
    fn schema() -> Schema {
        Schema::typed(
            "OverlayContext",
            TsType::Object(vec![TsField::required("visible", TsType::Boolean)]),
        )
    }
}

#[derive(TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct ActionActor {
    actor_id: u64,
    display_name: String,
}

#[derive(TsSchema)]
#[serde(rename_all = "camelCase")]
#[allow(dead_code)]
struct RecordActionInput {
    action_id: String,
    #[serde(flatten)]
    actor: ActionActor,
    confirmed: bool,
}

impl HostContract for RecordAction {
    const NAME: &'static str = "action.record";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["action", "record"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for RecordAction {
    type Input = RecordActionInput;
    type Output = String;
}

impl HostFunction for RecordAction {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(format!("{}:{}", input.action_id, input.actor.actor_id))
    }
}

#[test]
fn registry_supports_fluent_contract_registration() {
    let cache_dir = TestCacheDir::new("host-registry-api");
    let engine = Engine::new(&cache_dir.engine_options()).expect("create engine");

    engine
        .registry()
        .callback::<ScoreUpdate>()
        .and_then(|registry| registry.function::<FindUser>())
        .and_then(|registry| registry.context::<OverlayContext>())
        .expect("register host contracts");

    let function = engine
        .registry()
        .get(FindUser::NAME)
        .expect("get function")
        .expect("function descriptor");
    let callback = engine
        .registry()
        .get(ScoreUpdate::NAME)
        .expect("get callback")
        .expect("callback descriptor");
    let context = engine
        .registry()
        .get(OverlayContext::NAME)
        .expect("get context")
        .expect("context descriptor");

    assert_eq!(function.kind, HostContractKind::Function);
    assert_eq!(function.schema.name.as_deref(), Some("FindUserInput"));
    let function_metadata = function.function.expect("function metadata");
    assert_eq!(
        function_metadata.input_schema.name.as_deref(),
        Some("FindUserInput")
    );
    assert_eq!(
        function_metadata.output_schema.name.as_deref(),
        Some("FindUserOutput")
    );
    assert_eq!(callback.kind, HostContractKind::Callback);
    assert_eq!(callback.schema.name.as_deref(), Some("ScoreUpdatePayload"));
    assert_eq!(context.kind, HostContractKind::Context);
    let dts = engine.registry().dts().expect("render dts");
    support::assert_tsc(
        "contracts-dts",
        &format!("{dts}\nconst found: string = user.find(1);\n"),
        None,
    );
    support::assert_tsc(
        "contracts-dts-misuse",
        &format!("{dts}\nuser.find(\"one\");\n"),
        Some("TS2345"),
    );
}

#[test]
fn generated_sdk_uses_flattened_derived_input_schema() {
    let engine = support::engine();
    engine
        .registry()
        .function::<RecordAction>()
        .expect("register flattened host contract");
    let sdk = engine.registry().sdk().expect("render sdk");

    support::assert_tsc(
        "flattened-sdk",
        &format!(
            "{sdk}\nconst recorded: string = action.record({{ actionId: \"act-1\", actorId: 7, displayName: \"Nami\", confirmed: true }});\n"
        ),
        None,
    );
    support::assert_tsc(
        "flattened-sdk-misuse",
        &format!(
            "{sdk}\naction.record({{ actionId: \"act-1\", actorId: 7, displayName: \"Nami\", confirmed: true, actor: {{ actorId: 7, displayName: \"Nami\" }} }});\n"
        ),
        Some("TS2561"),
    );
}

#[test]
fn host_context_is_declarative_sdk_surface_only() {
    let engine = support::engine();
    engine
        .registry()
        .context::<OverlayContext>()
        .expect("register host context");
    let sdk = engine.registry().sdk().expect("render sdk");

    support::assert_tsc(
        "host-context-declarative",
        &format!("{sdk}\nconst visible: boolean = overlay.visible;\n"),
        None,
    );
    support::assert_tsc(
        "host-context-declarative-misuse",
        &format!("{sdk}\nconst visible: number = overlay.visible;\n"),
        Some("TS2322"),
    );
}

#[test]
fn importing_a_host_context_fails_at_load_instead_of_binding_undefined() {
    let cache_dir = TestCacheDir::new("host-context-import");
    let mut engine = Engine::new(&cache_dir.engine_options()).expect("create engine");
    engine
        .registry()
        .context::<OverlayContext>()
        .and_then(|registry| registry.function::<FindUser>())
        .expect("register host contracts");

    let loaded = engine.load_script(
        "overlay-reader",
        r#"
        import { overlay } from "test";

        export function overlayType() {
          return typeof overlay;
        }
        "#,
    );
    let result = engine.call::<Value>("overlay-reader", "overlayType", ());

    let Err(error) = loaded else {
        panic!("context import loaded and evaluated to {result:?}");
    };
    assert!(
        matches!(&error, EngineError::Execution { details } if details.contains("overlay")),
        "unexpected error: {error}"
    );
}

#[test]
fn registry_writes_sdk_files() {
    let cache_dir = TestCacheDir::new("host-registry-sdk-files");
    let output_dir = cache_dir.path().join("generated");
    let engine = Engine::new(&cache_dir.engine_options()).expect("create engine");

    engine
        .registry()
        .callback::<ScoreUpdate>()
        .and_then(|registry| registry.function::<FindUser>())
        .expect("register host contracts");
    let written = engine
        .registry()
        .write_sdk_files(&output_dir)
        .expect("write sdk files");

    assert!(written.types_path.is_file());
    assert!(written.sdk_path.is_file());
    assert_eq!(written.types_path, output_dir.join("manivelle.d.ts"));
    assert_eq!(written.sdk_path, output_dir.join("manivelle.sdk.ts"));
}

#[cfg(feature = "cli")]
#[test]
fn manivelle_sdk_command_writes_files_from_descriptor_json() {
    let cache_dir = TestCacheDir::new("host-registry-sdk-bin");
    let descriptors_path = cache_dir.path().join("descriptors.json");
    let output_dir = cache_dir.path().join("out");
    let registry = manivelle::InMemoryHostContractRegistry::new();
    registry
        .callback::<ScoreUpdate>()
        .and_then(|registry| registry.function::<FindUser>())
        .expect("register host contracts");
    std::fs::write(
        &descriptors_path,
        serde_json::to_string(&registry.descriptors().expect("descriptors"))
            .expect("serialize descriptors"),
    )
    .expect("write descriptors");

    let output = Command::new(env!("CARGO_BIN_EXE_manivelle"))
        .arg("sdk")
        .arg(&descriptors_path)
        .arg(&output_dir)
        .output()
        .expect("run manivelle");

    assert!(
        output.status.success(),
        "manivelle sdk failed\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(output_dir.join("manivelle.d.ts").exists());
    assert!(output_dir.join("manivelle.sdk.ts").exists());
}

#[test]
fn generated_sdk_typechecks_when_tsc_is_available() {
    let engine = support::engine();
    engine
        .registry()
        .callback::<ScoreUpdate>()
        .and_then(|registry| registry.function::<FindUser>())
        .and_then(|registry| registry.context::<OverlayContext>())
        .expect("register host contracts");
    let sdk = engine.registry().sdk().expect("render sdk");
    support::assert_tsc("host-registry-sdk-tsc", &sdk_usage_source(&sdk), None);
}

fn sdk_usage_source(sdk: &str) -> String {
    format!(
        "{sdk}\n\
const found = user.find(1);\n\
found.toUpperCase();\n\
const foundFromDynamicCall = call(\"user.find\", 3);\n\
foundFromDynamicCall.toUpperCase();\n\
overlay.visible.valueOf();\n\
events.score.update(event => {{\n\
  event.combo.toFixed();\n\
}});\n\
const foundFromAggregate = manivelleSdk.functions.user.find(2);\n\
foundFromAggregate.toUpperCase();\n\
const foundFromAggregateCall = manivelleSdk.call(\"user.find\", 4);\n\
foundFromAggregateCall.toUpperCase();\n\
const scorePayload = models.ScoreUpdatePayload.create({{ combo: 8 }});\n\
scorePayload.combo.toFixed();\n\
if (ScoreUpdatePayloadModel.is({{ combo: 13 }})) {{\n\
  ScoreUpdatePayloadModel.create({{ combo: 13 }}).combo.toFixed();\n\
}}\n\
if (models.ScoreUpdatePayload.is({{ combo: 14 }})) {{\n\
  models.ScoreUpdatePayload.create({{ combo: 14 }}).combo.toFixed();\n\
}}\n\
const scorePayloadModel = models.ScoreUpdatePayload.wrap({{ combo: 10 }});\n\
scorePayloadModel.value.combo.toFixed();\n\
scorePayloadModel.toJSON().combo.toFixed();\n\
const directScorePayloadModel = new ScoreUpdatePayloadModel({{ combo: 11 }});\n\
directScorePayloadModel.valueOf().combo.toFixed();\n\
const scorePayloadFromAggregate = manivelleSdk.models.ScoreUpdatePayload.create({{ combo: 9 }});\n\
scorePayloadFromAggregate.combo.toFixed();\n\
const scorePayloadModelFromAggregate = manivelleSdk.models.ScoreUpdatePayload.wrap({{ combo: 12 }});\n\
scorePayloadModelFromAggregate.toJSON().combo.toFixed();\n\
manivelleSdk.contexts.overlay.visible.valueOf();\n\
manivelleSdk.events.score.update(event => {{\n\
  event.combo.toFixed();\n\
}});\n\
score.onUpdate(event => {{\n\
  event.combo.toFixed();\n\
}});\n\
manivelleSdk.ctx.on(\"score.update\", event => {{\n\
  event.combo.toFixed();\n\
}});\n"
    )
}

#[test]
fn host_contract_validation_uses_flattened_derived_input_schema() {
    let cache_dir = TestCacheDir::new("host-contract-flatten-validation");
    let mut options = cache_dir.engine_options();
    options.contract_validation = ContractValidation::Inputs;
    options.unknown_field_validation = UnknownFieldValidation::Reject;
    let mut engine = Engine::new(&options).expect("create engine");

    engine
        .registry()
        .function::<RecordAction>()
        .expect("register flattened host function");
    engine
        .load_script(
            "action-script",
            r#"
        export function recordGood() {
          return action.record({
            actionId: "act-1",
            actorId: 7,
            displayName: "Nami",
            confirmed: true,
          });
        }

        export function recordBad() {
          return action.record({
            actionId: "act-1",
            actorId: 7,
            displayName: "Nami",
            confirmed: true,
            actor: { actorId: 7, displayName: "Nami" },
          });
        }
        "#,
        )
        .expect("load action script");

    let result = engine
        .call::<Value>("action-script", "recordGood", ())
        .expect("flattened input should validate");
    let error = engine
        .call::<Value>("action-script", "recordBad", ())
        .expect_err("nested pre-flatten field should be unknown");

    assert_eq!(result, json!("act-1:7"));
    assert!(matches!(
        error,
        EngineError::Execution { details }
            if details.contains("action.record")
                && details.contains("input validation failed")
                && details.contains("$.actor: unknown field")
    ));
}
