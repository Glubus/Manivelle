//! Contract validation (`VmOptions::contract_validation`): inputs and outputs of host
//! functions are checked against the schemas their contracts declare, for static
//! functions, closures, caller-aware closures and async functions alike.
//!
//! The contracts here declare their schema by hand on a type that crosses as a free
//! `serde_json::Value`, so a script can send values the schema rejects. They need no
//! `derive` feature.

use std::sync::{Arc, Mutex};

use manivelle::{
    Engine, HostContract, HostContractKind, HostFunction, HostFunctionSignature, HostResolver,
    JsDecode, JsEncode, NativeBytes, Schema, TsField, TsSchema, TsType, VmContractValidation,
    VmError, VmOptions, VmUnknownFieldValidation,
};
use serde_json::{Value, json};

/// Defines a type that crosses as an arbitrary JSON value but declares `$ts_type` (and the
/// named `$dependencies`) as its schema, so validation is checked against the declared
/// schema rather than against the Rust type.
macro_rules! declared {
    ($name:ident, $ts_type:expr, $dependencies:expr) => {
        #[derive(Debug, Clone, PartialEq)]
        struct $name(Value);

        impl TsSchema for $name {
            fn schema_name() -> &'static str {
                stringify!($name)
            }

            fn ts_type() -> TsType {
                $ts_type
            }

            fn schema_dependencies() -> Vec<Schema> {
                $dependencies
            }
        }

        impl JsEncode for $name {
            fn encode_js<'js>(
                &self,
                ctx: &manivelle::js::Ctx<'js>,
            ) -> manivelle::js::Result<manivelle::js::Value<'js>> {
                self.0.encode_js(ctx)
            }
        }

        impl JsDecode for $name {
            fn decode_js<'js>(
                ctx: &manivelle::js::Ctx<'js>,
                value: manivelle::js::Value<'js>,
            ) -> manivelle::js::Result<Self> {
                Value::decode_js(ctx, value).map($name)
            }
        }
    };
}

declared!(
    IdObject,
    TsType::Object(vec![TsField::required("id", TsType::Number)]),
    Vec::new()
);
declared!(
    UserRef,
    TsType::Object(vec![TsField::required(
        "user_id",
        TsType::TypeRef(String::from("UserId")),
    )]),
    vec![Schema::typed("UserId", TsType::Number)]
);
declared!(Anything, TsType::Json, Vec::new());
declared!(NumberValue, TsType::Number, Vec::new());
declared!(
    TagResult,
    TsType::Object(vec![TsField::required("label", TsType::String)]),
    Vec::new()
);

/// Defines function contract `$name` named `$contract`, exposed as `$module.$export`.
macro_rules! function_contract {
    ($name:ident, $contract:literal, $module:literal, $export:literal, $input:ty => $output:ty) => {
        struct $name;

        impl HostContract for $name {
            const NAME: &'static str = $contract;
            const IMPORT_MODULE: &'static str = "test";
            const EXPORT_PATH: &'static [&'static str] = &[$module, $export];

            fn kind() -> HostContractKind {
                HostContractKind::Function
            }
        }

        impl HostFunctionSignature for $name {
            type Input = $input;
            type Output = $output;
        }
    };
}

function_contract!(Echo, "validation.echo", "validation", "echo", IdObject => IdObject);
function_contract!(EchoRef, "validation.echoRef", "validation", "echoRef", UserRef => Anything);
function_contract!(
    BadOutput, "validation.badOutput", "validation", "badOutput", NumberValue => NumberValue
);
function_contract!(Tag, "tags.add", "tags", "add", IdObject => TagResult);
function_contract!(BytesEcho, "bytes.echo", "bytes", "echo", NativeBytes => NativeBytes);
function_contract!(NumberEcho, "numbers.echo", "numbers", "echo", f64 => f64);

impl HostFunction for Echo {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(input)
    }
}

impl HostFunction for EchoRef {
    fn call(input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(Anything(input.0))
    }
}

impl HostFunction for BadOutput {
    fn call(_input: Self::Input) -> Result<Self::Output, VmError> {
        Ok(NumberValue(json!("not-a-number")))
    }
}

const SCRIPT: &str = r#"
import { validation } from "test";

export function echo(input: unknown) { return validation.echo(input as any); }
export function echoRef(input: unknown) { return validation.echoRef(input as any); }
export function badOutput(input: unknown) { return validation.badOutput(input as any); }
"#;

fn engine(validation: VmContractValidation) -> Engine {
    Engine::new(&VmOptions {
        contract_validation: validation,
        ..VmOptions::default()
    })
    .expect("create engine")
}

fn engine_rejecting_unknown_fields() -> Engine {
    Engine::new(&VmOptions {
        contract_validation: VmContractValidation::Inputs,
        unknown_field_validation: VmUnknownFieldValidation::Reject,
        ..VmOptions::default()
    })
    .expect("create engine")
}

/// An engine with `validation` and the `validation.*` script loaded as `validation`.
fn loaded(validation: VmContractValidation) -> Engine {
    let mut engine = engine(validation);
    engine
        .registry()
        .function::<Echo>()
        .and_then(|registry| registry.function::<EchoRef>())
        .and_then(|registry| registry.function::<BadOutput>())
        .expect("register contracts");
    engine
        .load_script("validation", SCRIPT)
        .expect("load validation script");
    engine
}

fn call(engine: &Engine, function: &str, input: Value) -> Result<Value, VmError> {
    engine.call::<Value>("validation", function, vec![input])
}

#[track_caller]
fn assert_fails_with(result: Result<Value, VmError>, expected: &[&str]) {
    match result {
        Err(VmError::Execution { details }) => {
            for part in expected {
                assert!(details.contains(part), "`{part}` missing from: {details}");
            }
        }
        other => panic!("expected a validation failure, got {other:?}"),
    }
}

#[test]
fn an_input_outside_the_schema_fails_the_call_and_names_the_contract_and_the_field() {
    let engine = loaded(VmContractValidation::Inputs);

    let result = call(&engine, "echo", json!({ "id": "bad" }));

    assert_fails_with(
        result,
        &["validation.echo", "input validation failed", "$.id"],
    );
}

#[test]
fn an_input_inside_the_schema_passes_through_unchanged() {
    let engine = loaded(VmContractValidation::InputsAndOutputs);

    let result = call(&engine, "echo", json!({ "id": 7 }));

    assert_eq!(result.expect("valid call"), json!({ "id": 7 }));
}

#[test]
fn a_named_type_a_schema_refers_to_is_resolved_before_checking() {
    let engine = loaded(VmContractValidation::Inputs);

    let result = call(&engine, "echoRef", json!({ "user_id": "bad" }));

    assert_fails_with(
        result,
        &[
            "validation.echoRef",
            "input validation failed",
            "$.user_id: expected number, got string",
        ],
    );
}

#[test]
fn validation_is_off_by_default() {
    let engine = loaded(VmContractValidation::Disabled);

    let result = call(&engine, "echo", json!({ "id": "bad" }));

    assert_eq!(result.expect("not validated"), json!({ "id": "bad" }));
}

#[test]
fn outputs_are_only_checked_when_the_policy_asks_for_them() {
    let inputs_only = loaded(VmContractValidation::Inputs);
    let both = loaded(VmContractValidation::InputsAndOutputs);

    let unchecked = call(&inputs_only, "badOutput", json!(1));
    let checked = call(&both, "badOutput", json!(1));

    assert_eq!(
        unchecked.expect("output not checked"),
        json!("not-a-number")
    );
    assert_fails_with(
        checked,
        &["validation.badOutput", "output validation failed"],
    );
}

#[test]
fn unknown_input_fields_are_allowed_unless_the_policy_rejects_them() {
    let lenient = loaded(VmContractValidation::Inputs);
    let mut strict = engine_rejecting_unknown_fields();
    strict
        .registry()
        .function::<Echo>()
        .expect("register contract");
    strict
        .load_script("validation", SCRIPT)
        .expect("load validation script");
    let input = json!({ "id": 1, "extra": true });

    let allowed = call(&lenient, "echo", input.clone());
    let rejected = call(&strict, "echo", input.clone());

    assert_eq!(allowed.expect("unknown field allowed"), input);
    assert_fails_with(
        rejected,
        &[
            "validation.echo",
            "input validation failed",
            "$.extra: unknown field",
        ],
    );
}

#[test]
fn a_reloaded_script_is_still_validated() {
    let mut engine = loaded(VmContractValidation::Inputs);
    engine
        .load_script("validation", SCRIPT)
        .expect("reload validation script");

    let result = call(&engine, "echo", json!({ "id": "bad" }));

    assert_fails_with(result, &["input validation failed"]);
}

/// Inputs the closures of [`closure_engine`] received.
type Handled = Arc<Mutex<Vec<Value>>>;

/// An engine validating inputs and outputs whose `tags.add` is a closure, with the inputs
/// it received. Its label is `7` (a number, against a declared string) when the input id
/// is `0`, and the calling script's id (`closure` without a caller) otherwise.
fn closure_engine(with_caller: bool) -> (Engine, Handled) {
    let mut engine = engine(VmContractValidation::InputsAndOutputs);
    let handled = Handled::default();
    let seen = Arc::clone(&handled);
    let answer = |input: &IdObject, script: &str| {
        let label = if input.0["id"] == json!(0) {
            json!(7)
        } else {
            json!(script)
        };
        TagResult(json!({ "label": label }))
    };
    if with_caller {
        engine
            .registry()
            .function_with_caller::<Tag>(move |caller, input| {
                seen.lock().expect("handled").push(input.0.clone());
                Ok(answer(&input, caller.script_id()))
            })
            .expect("register closure");
    } else {
        engine
            .registry()
            .function_with::<Tag>(move |input| {
                seen.lock().expect("handled").push(input.0.clone());
                Ok(answer(&input, "closure"))
            })
            .expect("register closure");
    }
    engine
        .load_script(
            "tagger",
            r#"
            import { tags } from "test";
            export function valid() { return tags.add({ id: 7 } as any).label; }
            export function badInput() { return tags.add({ id: "seven" } as any); }
            export function badOutput() { return tags.add({ id: 0 } as any); }
            "#,
        )
        .expect("load script");
    (engine, handled)
}

#[test]
fn closures_are_validated_and_an_invalid_input_never_reaches_them() {
    for with_caller in [false, true] {
        let (engine, handled) = closure_engine(with_caller);

        let valid: String = engine.call("tagger", "valid", ()).expect("valid call");
        let bad_input = engine.call::<Value>("tagger", "badInput", ());
        let bad_output = engine.call::<Value>("tagger", "badOutput", ());

        assert_eq!(valid, if with_caller { "tagger" } else { "closure" });
        assert_fails_with(bad_input, &["tags.add", "input validation failed"]);
        assert_fails_with(bad_output, &["tags.add", "output validation failed"]);
        assert_eq!(
            *handled.lock().expect("handled"),
            [json!({ "id": 7 }), json!({ "id": 0 })],
            "with_caller = {with_caller}"
        );
    }
}

type Calls = Arc<Mutex<Vec<(Value, HostResolver<TagResult>)>>>;

/// An engine validating as `validation` says whose async `tags.add` keeps every call's
/// input and resolver for the test to settle, and a script awaiting it.
fn async_engine(validation: VmContractValidation) -> (Engine, Calls) {
    let mut engine = engine(validation);
    let calls = Calls::default();
    let kept = Arc::clone(&calls);
    engine
        .registry()
        .async_function_with::<Tag>(move |input, resolver| {
            kept.lock().expect("calls").push((input.0, resolver));
            Ok(())
        })
        .expect("register async function");
    engine
        .load_script(
            "tagger",
            r#"
            import { tags } from "test";
            export async function add(id: unknown): Promise<string> {
              try { return String((await tags.add({ id } as any)).label); }
              catch (error) { return `rejected: ${(error as Error).message}`; }
            }
            "#,
        )
        .expect("load script");
    (engine, calls)
}

fn answer(label: Value) -> TagResult {
    TagResult(json!({ "label": label }))
}

#[test]
fn an_async_input_outside_the_schema_rejects_the_promise_without_starting_the_call() {
    let (engine, calls) = async_engine(VmContractValidation::Inputs);

    let message: String = engine.call("tagger", "add", ("seven",)).expect("call");

    assert!(message.contains("input validation failed"), "{message}");
    assert!(
        calls.lock().expect("calls").is_empty(),
        "an invalid input never reaches the handler"
    );
}

#[test]
fn an_async_output_outside_the_schema_rejects_the_promise_when_the_engine_converts_it() {
    let (engine, calls) = async_engine(VmContractValidation::InputsAndOutputs);
    let valid = engine
        .call_deferred::<String>("tagger", "add", (7,))
        .expect("start valid call");
    let invalid = engine
        .call_deferred::<String>("tagger", "add", (8,))
        .expect("start call answered badly");
    let (invalid_input, invalid_resolver) = calls.lock().expect("calls").pop().expect("call 8");
    let (valid_input, valid_resolver) = calls.lock().expect("calls").pop().expect("call 7");

    valid_resolver
        .resolve(answer(json!("seven")))
        .expect("resolve");
    invalid_resolver
        .resolve(answer(json!(8)))
        .expect("the host cannot see the output is invalid");
    engine.pump().expect("pump");

    assert_eq!(valid_input, json!({ "id": 7 }));
    assert_eq!(invalid_input, json!({ "id": 8 }));
    assert_eq!(valid.take().expect("finished").expect("value"), "seven");
    let rejected = invalid
        .take()
        .expect("finished")
        .expect("caught in the script");
    assert!(rejected.contains("output validation failed"), "{rejected}");
}

#[test]
fn an_async_output_is_not_checked_when_only_inputs_are() {
    let (engine, calls) = async_engine(VmContractValidation::Inputs);
    let pending = engine
        .call_deferred::<String>("tagger", "add", (8,))
        .expect("start call");
    let (_, resolver) = calls.lock().expect("calls").pop().expect("call 8");

    resolver.resolve(answer(json!(8))).expect("resolve");
    engine.pump().expect("pump");

    assert_eq!(pending.take().expect("finished").expect("value"), "8");
}

/// An engine validating inputs and outputs of `bytes.echo` and `numbers.echo`, with the
/// script calling them loaded as `natives`.
fn natives_engine() -> Engine {
    let mut engine = engine(VmContractValidation::InputsAndOutputs);
    engine
        .registry()
        .function_with::<BytesEcho>(Ok)
        .and_then(|registry| registry.function_with::<NumberEcho>(Ok))
        .expect("register native contracts");
    engine
        .load_script(
            "natives",
            r#"
            import { bytes, numbers } from "test";
            export function echoView() {
                const out = bytes.echo(new Uint8Array([1, 2, 3]));
                return out instanceof Uint8Array ? Array.from(out) : null;
            }
            export function echoBuffer() {
                return Array.from(bytes.echo(new Uint8Array([4, 5]).buffer as any));
            }
            export function echoObject() { return bytes.echo({ 0: 1 } as any); }
            export function echoNonFinite() {
                return [numbers.echo(NaN), numbers.echo(Infinity), numbers.echo(-Infinity)]
                    .map(String);
            }
            export function echoString() { return numbers.echo("1" as any); }
            "#,
        )
        .expect("load natives script");
    engine
}

#[test]
fn native_bytes_and_non_finite_numbers_pass_input_and_output_validation() {
    let engine = natives_engine();

    let view: Vec<u8> = engine.call("natives", "echoView", ()).expect("echo view");
    let buffer: Vec<u8> = engine
        .call("natives", "echoBuffer", ())
        .expect("echo buffer");
    let non_finite: Vec<String> = engine
        .call("natives", "echoNonFinite", ())
        .expect("echo non-finite numbers");

    assert_eq!(view, [1, 2, 3]);
    assert_eq!(buffer, [4, 5]);
    assert_eq!(non_finite, ["NaN", "Infinity", "-Infinity"]);
}

#[test]
fn byte_and_number_schemas_still_reject_other_values() {
    let engine = natives_engine();

    let object = engine.call::<Value>("natives", "echoObject", ());
    let string = engine.call::<Value>("natives", "echoString", ());

    assert_fails_with(
        object,
        &["input validation failed", "expected array, got object"],
    );
    assert_fails_with(
        string,
        &["input validation failed", "expected number, got string"],
    );
}
