//! Host functions and callbacks used by projects and scripts: bridges, native bytes,
//! errors with their stacks, and reloads that rewire the host.

mod support;

use std::fs;

use manivelle::{
    Engine, EngineError, HostCallback, HostContract, HostContractKind, HostFunction,
    HostFunctionSignature, NativeBytes,
};
use serde_json::{Value, json};
use support::{TestCacheDir, call, engine, fixture};

const HOST_BRIDGE_SCRIPT: &str = include_str!("projects/host_bridge/main.ts");
const LAZY_HOST_BRIDGE_SCRIPT: &str = include_str!("projects/lazy_host_bridge/main.ts");
const THROWING_SCRIPT: &str = include_str!("projects/throwing/main.ts");
const PROJECT_MAIN_V1: &str = include_str!("projects/reloadable_project/main_v1.ts");
const PROJECT_MAIN_V2: &str = include_str!("projects/reloadable_project/main_v2.ts");

struct FindUser;
struct FindInvoice;
struct ReadNativeBytes;
struct ScoreUpdate;

impl HostContract for FindUser {
    const NAME: &'static str = "user.find";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["user", "find"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindUser {
    type Input = u64;
    type Output = String;
}

impl HostFunction for FindUser {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(format!("user-{input}"))
    }
}

impl HostContract for FindInvoice {
    const NAME: &'static str = "billing.invoice.find";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["billing", "invoice", "find"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindInvoice {
    type Input = u64;
    type Output = String;
}

impl HostFunction for FindInvoice {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(format!("invoice-{input}"))
    }
}

impl HostContract for ReadNativeBytes {
    const NAME: &'static str = "bench.bytes.native";

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for ReadNativeBytes {
    type Input = Value;
    type Output = NativeBytes;
}

impl HostFunction for ReadNativeBytes {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        let byte_count = input.get("byteCount").and_then(Value::as_u64).unwrap_or(0) as usize;
        Ok(NativeBytes::new(
            (0..byte_count)
                .map(|index| (index % 251) as u8)
                .collect::<Vec<_>>(),
        ))
    }
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
    type Payload = Value;
}

#[test]
fn script_calls_a_host_function_through_its_import_namespace() {
    let mut engine = engine();
    engine
        .registry()
        .function::<FindUser>()
        .expect("register host function");

    engine
        .load_script("host-bridge", HOST_BRIDGE_SCRIPT)
        .expect("load host bridge script");

    assert_eq!(
        call(
            &engine,
            "host-bridge",
            "lookupWithNamespace",
            vec![json!(7)]
        ),
        json!("user-7")
    );
}

#[test]
fn host_import_namespace_exposes_only_registered_functions() {
    let mut engine = engine();
    engine
        .registry()
        .function::<FindUser>()
        .and_then(|registry| registry.function::<FindInvoice>())
        .expect("register host functions");

    engine
        .load_script("lazy-host-bridge", LAZY_HOST_BRIDGE_SCRIPT)
        .expect("load lazy host bridge script");

    assert_eq!(
        call(
            &engine,
            "lazy-host-bridge",
            "inspectBindings",
            vec![json!(9)]
        ),
        json!({
            "userFindType": "function",
            "missingType": "undefined",
            "invoice": "invoice-9",
        })
    );
}

#[test]
fn typed_host_function_returns_native_bytes_as_uint8array() {
    let mut engine = engine();
    engine
        .registry()
        .function::<ReadNativeBytes>()
        .expect("register native bytes host function");
    let declarations = engine.registry().dts().expect("render declarations");
    engine
        .load_script(
            "native-bytes",
            r#"
export function inspectNativeBytes(byteCount: number) {
  const bytes = (globalThis as any).__host.callValue("bench.bytes.native", { byteCount });
  return {
    isView: ArrayBuffer.isView(bytes),
    constructorName: bytes.constructor.name,
    length: bytes.length,
    fourth: bytes[3],
    sampled: bytes[0] + bytes[4096],
  };
}
"#,
        )
        .expect("load native bytes script");

    assert!(
        declarations.contains("type NativeBytes = Uint8Array;"),
        "{declarations}"
    );
    assert_eq!(
        call(
            &engine,
            "native-bytes",
            "inspectNativeBytes",
            vec![json!(8192)]
        ),
        json!({
            "isView": true,
            "constructorName": "Uint8Array",
            "length": 8192,
            "fourth": 3,
            "sampled": 80,
        })
    );
}

#[test]
fn script_errors_carry_the_stack_of_the_throwing_function() {
    let mut engine = engine();
    engine
        .load_script("throwing", THROWING_SCRIPT)
        .expect("load throwing script");

    let result = engine.call::<Value>("throwing", "explode", ());

    assert!(
        matches!(&result, Err(EngineError::Execution { details })
            if details.contains("boom from script") && details.contains("explode")),
        "{result:?}"
    );
}

#[test]
fn project_reload_replaces_the_dependency_graph_and_its_event_handlers() {
    let cache = TestCacheDir::new("project-reload-graph");
    let mut engine = Engine::new(&cache.engine_options()).expect("create engine");
    engine
        .registry()
        .callback::<ScoreUpdate>()
        .expect("register score update callback");
    let project_root = cache.path().join("project");
    let entry_path = project_root.join("main.ts");
    let value_path = project_root.join("value.ts");
    fs::create_dir_all(&project_root).expect("create project");
    fs::write(&entry_path, PROJECT_MAIN_V1).expect("write first entry");
    fs::write(&value_path, "export const current = 1;\n").expect("write first value");

    engine
        .load_project("project", &entry_path)
        .expect("load first project");
    let first_delivered = engine
        .emit("score.update", &json!({ "combo": 5 }))
        .expect("emit first event");
    let first_score = call(&engine, "project", "readScore", Vec::new());

    fs::write(&entry_path, PROJECT_MAIN_V2).expect("write second entry");
    fs::write(&value_path, "export const current = 2;\n").expect("write second value");
    fs::write(project_root.join("extra.ts"), "export const offset = 10;\n")
        .expect("write extra module");
    engine
        .load_project("project", &entry_path)
        .expect("reload project");
    let second_delivered = engine
        .emit("score.update", &json!({ "combo": 5 }))
        .expect("emit second event");
    let second_value = call(&engine, "project", "read", Vec::new());
    let second_score = call(&engine, "project", "readScore", Vec::new());

    assert_eq!((first_delivered, first_score), (1, json!(6)));
    assert_eq!(
        (second_delivered, second_value, second_score),
        (1, json!(12), json!(17))
    );
}

#[test]
fn realistic_mod_pack_uses_aliases_host_calls_events_and_state() {
    let mut engine = engine();
    engine
        .registry()
        .function::<FindUser>()
        .and_then(|registry| registry.callback::<ScoreUpdate>())
        .expect("register host contracts");

    engine
        .load_project("raid-mod", fixture("realistic_mod_pack/src/main.ts"))
        .expect("load realistic mod pack");
    let damage = call(
        &engine,
        "raid-mod",
        "simulateDamage",
        vec![json!({ "playerId": 7, "baseDamage": 9, "critical": true })],
    );
    let invoice = call(
        &engine,
        "raid-mod",
        "invoiceFor",
        vec![json!(7), json!(12.5)],
    );
    let delivered = engine
        .emit("score.update", &json!({ "combo": 5 }))
        .expect("emit score event");
    let state = call(&engine, "raid-mod", "readState", Vec::new());

    assert_eq!(
        damage,
        json!({ "playerId": 7, "playerName": "user-7", "damage": 18, "score": 18 })
    );
    assert_eq!(invoice, json!("invoice:user-7:12.50"));
    assert_eq!(delivered, 1);
    assert_eq!(
        state,
        json!({
            "score": 23,
            "overlay": {
                "visible": true,
                "messages": ["damage:user-7:18", "combo:5"],
            },
            "session": { "id": "raid-night-01", "ticks": 1 },
        })
    );
}
