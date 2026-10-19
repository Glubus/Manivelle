#![cfg(feature = "derive")]

mod support;

use manivelle::{
    Engine, EngineError, EngineOptions, HostCallback, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, TsSchema,
};
use serde::{Deserialize, Serialize};

const DOGFOOD_SCRIPT: &str = r#"
let lastFound = "none";

ctx.on("user.found", event => {
  lastFound = `${event.displayName}:${event.roles.join(",")}`;
});

export function lookup() {
  const result = user.find({ userId: 7, includeRoles: true });
  return `${result.displayName}:${result.roles.length}:${result.active}`;
}

export function observed() {
  return lastFound;
}
"#;

/// The dogfood script written against the generated SDK instead of the injected globals.
const DOGFOOD_SDK_SCRIPT: &str = r#"
let lastFound = "none";

user.onFound(event => {
  lastFound = `${event.displayName}:${event.roles.join(",")}`;
});

export function lookup(): string {
  const result = manivelleSdk.functions.user.find({ userId: 7, includeRoles: true });
  return `${result.displayName}:${result.roles.length}:${result.active}`;
}

export function observed(): string {
  return lastFound;
}
"#;

#[derive(Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct DogfoodFindUserInput {
    user_id: u64,
    include_roles: bool,
}

#[derive(Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct DogfoodFindUserOutput {
    user_id: u64,
    display_name: String,
    active: bool,
    roles: Vec<String>,
}

#[derive(Deserialize, Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct DogfoodUserFoundPayload {
    user_id: u64,
    display_name: String,
    roles: Vec<String>,
}

struct DogfoodFindUser;
struct DogfoodUserFound;

impl HostContract for DogfoodFindUser {
    const NAME: &'static str = "user.find";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for DogfoodFindUser {
    type Input = DogfoodFindUserInput;
    type Output = DogfoodFindUserOutput;
}

impl HostFunction for DogfoodFindUser {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(DogfoodFindUserOutput {
            user_id: input.user_id,
            display_name: format!("user-{}", input.user_id),
            active: true,
            roles: if input.include_roles {
                vec![String::from("admin"), String::from("editor")]
            } else {
                Vec::new()
            },
        })
    }
}

impl HostContract for DogfoodUserFound {
    const NAME: &'static str = "user.found";

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for DogfoodUserFound {
    type Payload = DogfoodUserFoundPayload;
}

#[test]
fn engine_runs_the_dogfood_script_through_injected_globals() {
    let mut engine = dogfood_engine();
    engine
        .load_script("dogfood", DOGFOOD_SCRIPT)
        .expect("load dogfood script");

    assert_engine_dogfood_round_trip(&engine);
}

#[test]
fn engine_runs_a_script_built_on_the_generated_sdk() {
    let mut engine = dogfood_engine();
    let sdk = engine.registry().sdk().expect("render generated SDK");
    typecheck_sdk_usage_when_tsc_is_available(&sdk);
    support::assert_tsc(
        "dogfood-sdk-misuse",
        &format!("{sdk}\nuser.find(\"nope\");\n"),
        Some("TS2345"),
    );

    engine
        .load_script("dogfood", &format!("{sdk}\n{DOGFOOD_SDK_SCRIPT}"))
        .expect("load SDK-based dogfood script");

    assert_engine_dogfood_round_trip(&engine);
}

fn dogfood_engine() -> Engine {
    let engine = Engine::new(&EngineOptions::default()).expect("create engine");
    engine
        .registry()
        .function::<DogfoodFindUser>()
        .and_then(|registry| registry.callback::<DogfoodUserFound>())
        .expect("register typed dogfood contracts");
    engine
}

fn assert_engine_dogfood_round_trip(engine: &Engine) {
    let lookup: String = engine
        .call("dogfood", "lookup", ())
        .expect("call dogfood lookup");
    let delivered = engine
        .emit(
            "user.found",
            &DogfoodUserFoundPayload {
                user_id: 7,
                display_name: String::from("user-7"),
                roles: vec![String::from("admin"), String::from("editor")],
            },
        )
        .expect("emit dogfood callback");
    let observed: String = engine
        .call("dogfood", "observed", ())
        .expect("read callback state");

    assert_eq!(lookup, "user-7:2:true");
    assert_eq!(delivered, 1);
    assert_eq!(observed, "user-7:admin,editor");
}

fn typecheck_sdk_usage_when_tsc_is_available(sdk: &str) {
    support::assert_tsc("dogfood-sdk-usage", &dogfood_sdk_usage_source(sdk), None);
}

fn dogfood_sdk_usage_source(sdk: &str) -> String {
    format!(
        "{sdk}\n\
const input = models.DogfoodFindUserInput.create({{ userId: 7, includeRoles: true }});\n\
const wrappedInput = models.DogfoodFindUserInput.wrap(input);\n\
const directWrappedInput = DogfoodFindUserInputModel.wrap(input);\n\
if (DogfoodFindUserInputModel.is(wrappedInput.value)) {{\n\
  wrappedInput.toJSON().userId.toFixed();\n\
}}\n\
if (models.DogfoodFindUserInput.is(directWrappedInput.valueOf())) {{\n\
  directWrappedInput.toJSON().includeRoles.valueOf();\n\
}}\n\
const found = user.find(input);\n\
found.displayName.toUpperCase();\n\
found.roles.map(role => role.toUpperCase());\n\
ctx.on(\"user.found\", event => {{\n\
  event.displayName.toUpperCase();\n\
  event.roles.length.toFixed();\n\
}});\n\
events.user.found(event => {{\n\
  event.userId.toFixed();\n\
}});\n\
user.onFound(event => {{\n\
  event.roles.map(role => role.toUpperCase());\n\
}});\n\
manivelleSdk.functions.user.find(input).active.valueOf();\n\
manivelleSdk.models.DogfoodUserFoundPayload.wrap({{ userId: 7, displayName: \"user-7\", roles: [\"admin\"] }}).toJSON().roles.length.toFixed();\n"
    )
}
