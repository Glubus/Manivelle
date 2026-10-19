use super::InMemoryHostContractRegistry;
use super::declarations::{
    ENV_MODULE_TYPE, EVENT_CONTEXT_TYPE, HOT_CONTEXT_TYPE, render_typescript_declarations,
};
use super::sdk::render_typescript_sdk;
use crate::contract::{
    HostCallback, HostCallbackDescriptor, HostContext, HostContract, HostContractAbi,
    HostContractDescriptor, HostContractKind, HostFunction, HostFunctionDescriptor,
    HostFunctionSignature, HostMetadata, Schema, TsEnumVariant, TsField, TsLiteral, TsRecordKey,
    TsSchema, TsType,
};
use crate::error::EngineError;

struct DemoFunction;
struct GeneratedFunction;
struct DemoCallback;
struct DemoContext;

impl HostContract for DemoFunction {
    const NAME: &'static str = "demo.function";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["demo", "function"];

    fn metadata() -> HostMetadata {
        HostMetadata {
            name: String::from(Self::NAME),
            tags: vec![String::from("function")],
        }
    }

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for DemoFunction {
    type Input = ();
    type Output = ();
}

impl HostFunction for DemoFunction {
    fn call(_input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(())
    }
}

impl HostContract for GeneratedFunction {
    const NAME: &'static str = "user.find";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["user", "find"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for GeneratedFunction {
    type Input = ();
    type Output = ();
}

impl HostFunction for GeneratedFunction {
    fn call(_input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(())
    }
}

impl HostContract for DemoCallback {
    const NAME: &'static str = "demo.callback";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["demo", "callback"];

    fn kind() -> HostContractKind {
        HostContractKind::Callback
    }
}

impl HostCallback for DemoCallback {
    type Payload = ();
}

impl HostContract for DemoContext {
    const NAME: &'static str = "demo.context";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["demo", "context"];

    fn kind() -> HostContractKind {
        HostContractKind::Context
    }
}

impl HostContext for DemoContext {
    fn schema() -> Schema {
        Schema::typed(
            "DemoContext",
            TsType::Object(vec![TsField::required("visible", TsType::Boolean)]),
        )
    }
}

#[test]
fn function_stores_descriptor() {
    let registry = InMemoryHostContractRegistry::new();

    registry.function::<DemoFunction>().unwrap();

    let descriptor = registry.get(DemoFunction::NAME).unwrap().unwrap();
    assert_eq!(descriptor.name, DemoFunction::NAME);
    assert_eq!(descriptor.kind, HostContractKind::Function);
    let function = descriptor.function.expect("function descriptor");
    assert_eq!(function.input_schema, <() as TsSchema>::schema());
    assert_eq!(function.output_schema, <() as TsSchema>::schema());
    assert_eq!(descriptor.schema, function.input_schema);
    assert!(matches!(
        descriptor.abi,
        HostContractAbi::Function { input, output, returns_promise: false }
            if input == <() as TsSchema>::schema() && output == <() as TsSchema>::schema()
    ));
}

#[test]
fn callback_stores_descriptor() {
    let registry = InMemoryHostContractRegistry::new();

    registry.callback::<DemoCallback>().unwrap();

    let descriptor = registry.get(DemoCallback::NAME).unwrap().unwrap();
    assert_eq!(descriptor.name, DemoCallback::NAME);
    assert_eq!(descriptor.kind, HostContractKind::Callback);
    let callback = descriptor.callback.unwrap();
    assert_eq!(callback.payload_schema, <() as TsSchema>::schema());
    assert_eq!(descriptor.schema, callback.payload_schema);
    assert!(matches!(
        descriptor.abi,
        HostContractAbi::Callback { payload, .. } if payload == <() as TsSchema>::schema()
    ));
}

#[test]
fn context_stores_descriptor() {
    let registry = InMemoryHostContractRegistry::new();

    registry.context::<DemoContext>().unwrap();

    let descriptor = registry.get(DemoContext::NAME).unwrap().unwrap();
    assert_eq!(descriptor.name, DemoContext::NAME);
    assert_eq!(descriptor.kind, HostContractKind::Context);
    assert!(matches!(
        descriptor.abi,
        HostContractAbi::Context { schema } if schema.name == "DemoContext"
    ));
}

#[test]
fn fluent_registration_chains_contracts() {
    let registry = InMemoryHostContractRegistry::new();

    registry
        .callback::<DemoCallback>()
        .and_then(|registry| registry.function::<DemoFunction>())
        .and_then(|registry| registry.context::<DemoContext>())
        .unwrap();

    assert!(registry.get(DemoCallback::NAME).unwrap().is_some());
    assert!(registry.get(DemoFunction::NAME).unwrap().is_some());
    assert!(registry.get(DemoContext::NAME).unwrap().is_some());
}

#[test]
fn descriptors_returns_sorted_by_name() {
    let registry = InMemoryHostContractRegistry::new();
    registry.context::<DemoContext>().unwrap();
    registry.function::<DemoFunction>().unwrap();
    registry.callback::<DemoCallback>().unwrap();

    let names = registry
        .descriptors()
        .unwrap()
        .into_iter()
        .map(|descriptor| descriptor.name)
        .collect::<Vec<_>>();

    assert_eq!(
        names,
        vec![
            String::from(DemoCallback::NAME),
            String::from(DemoContext::NAME),
            String::from(DemoFunction::NAME),
        ]
    );
}

struct ComplexGeneratedFunction;

impl HostContract for ComplexGeneratedFunction {
    const NAME: &'static str = "billing.invoice.create";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["billing", "invoice", "create"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for ComplexGeneratedFunction {
    type Input = ();
    type Output = ();
}

/// Input schema of [`ComplexGeneratedFunction`], hand-built to exercise every shape the
/// declaration renderer handles.
fn complex_input_schema() -> Schema {
    Schema::typed(
        "CreateInvoiceInput",
        TsType::Object(vec![
            TsField::required(
                "status",
                TsType::Union(vec![
                    TsType::Literal(TsLiteral::String(String::from("draft"))),
                    TsType::Literal(TsLiteral::String(String::from("paid"))),
                ]),
            ),
            TsField::optional(
                "metadata",
                TsType::Record {
                    key: TsRecordKey::String,
                    value: Box::new(TsType::Json),
                },
            ),
            TsField::required(
                "lines",
                TsType::Array(Box::new(TsType::Tuple(vec![
                    TsType::String,
                    TsType::Number,
                ]))),
            ),
            TsField::required(
                "paymentMethod",
                TsType::Enum {
                    tag: None,
                    variants: vec![TsEnumVariant::unit("card"), TsEnumVariant::unit("wire")],
                },
            ),
        ]),
    )
}

/// Output schema of [`ComplexGeneratedFunction`], hand-built to exercise every shape
/// the declaration renderer handles.
fn complex_output_schema() -> Schema {
    Schema::typed(
        "CreateInvoiceOutput",
        TsType::Object(vec![
            TsField::required("id", TsType::Nullable(Box::new(TsType::String))),
            TsField::required(
                "state",
                TsType::Enum {
                    tag: Some(String::from("kind")),
                    variants: vec![
                        TsEnumVariant::unit("created"),
                        TsEnumVariant::payload(
                            "failed",
                            vec![TsField::required("reason", TsType::String)],
                        ),
                    ],
                },
            ),
        ]),
    )
}

/// The descriptor a registration of function contract `C` produces, for schemas that no
/// Rust type declares, so renderer tests can feed it any shape.
fn function_descriptor<C: HostContract>(input: Schema, output: Schema) -> HostContractDescriptor {
    let function = HostFunctionDescriptor {
        input_schema: input.clone(),
        output_schema: output.clone(),
        returns_promise: false,
    };
    let mut descriptor = C::descriptor();
    descriptor.schema = input.clone();
    descriptor.abi = HostContractAbi::Function {
        input,
        output,
        returns_promise: false,
    };
    descriptor.function = Some(function);
    descriptor
}

/// The descriptor a registration of callback contract `C` produces, for a hand-built
/// payload schema.
fn callback_descriptor<C: HostContract>(payload: Schema) -> HostContractDescriptor {
    let callback = HostCallbackDescriptor {
        payload_schema: payload.clone(),
        reply_schema: None,
    };
    let mut descriptor = C::descriptor();
    descriptor.schema = payload.clone();
    descriptor.abi = HostContractAbi::Callback {
        payload,
        reply: None,
    };
    descriptor.callback = Some(callback);
    descriptor
}

fn find_user() -> HostContractDescriptor {
    function_descriptor::<GeneratedFunction>(
        Schema::typed("FindUserInput", TsType::Number),
        Schema::typed("FindUserOutput", TsType::String),
    )
}

fn demo_callback() -> HostContractDescriptor {
    callback_descriptor::<DemoCallback>(Schema::typed(
        "DemoCallbackPayload",
        TsType::Object(vec![TsField::required("combo", TsType::Number)]),
    ))
}

impl HostFunction for ComplexGeneratedFunction {
    fn call(_input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(())
    }
}

#[test]
fn dts_renders_nested_namespaces_and_complex_schema_types() {
    let declarations =
        render_typescript_declarations(&[function_descriptor::<ComplexGeneratedFunction>(
            complex_input_schema(),
            complex_output_schema(),
        )]);

    assert_eq!(
        declarations,
        format!(
            "type CreateInvoiceInput = {{ status: \"draft\" | \"paid\"; metadata?: Record<string, unknown>; lines: [string, number][]; paymentMethod: \"card\" | \"wire\"; }};\n\ntype CreateInvoiceOutput = {{ id: string | null; state: {{ kind: \"created\"; }} | {{ kind: \"failed\"; reason: string; }}; }};\n\ndeclare namespace billing {{\n  namespace invoice {{\n    export function create(input: CreateInvoiceInput): CreateInvoiceOutput;\n  }}\n}}\n\n{}",
            ctx_declaration(false)
        )
    );
}

#[test]
fn dts_is_generated_from_contract_schemas() {
    let declarations = render_typescript_declarations(&[
        demo_callback(),
        function_descriptor::<DemoFunction>(
            Schema::typed("DemoFunctionInput", TsType::Void),
            Schema::named("unknown"),
        ),
    ]);

    assert_eq!(
        declarations,
        format!(
            "type DemoCallbackPayload = {{ combo: number; }};\n\ntype DemoFunctionInput = void;\n\ndeclare namespace demo {{\n  export function function(input: DemoFunctionInput): unknown;\n}}\n\ntype HostEvents = {{\n  \"demo.callback\": DemoCallbackPayload;\n}};\n\ntype HostReplies = {{}};\n\n{}",
            ctx_declaration(true)
        )
    );
}

#[test]
fn dts_generates_function_declaration_from_contract_model() {
    let declarations = render_typescript_declarations(&[find_user()]);

    assert_eq!(
        declarations,
        format!(
            "type FindUserInput = number;\n\ntype FindUserOutput = string;\n\ndeclare namespace user {{\n  export function find(input: FindUserInput): FindUserOutput;\n}}\n\n{}",
            ctx_declaration(false)
        )
    );
}

/// The declarations' closing `ctx` section, typing `ctx.on` and `ctx.off` only when
/// events exist, and the `manivelle:env` module that exports it.
fn ctx_declaration(with_events: bool) -> String {
    let (event_context, events) = if with_events {
        (
            format!("{}\n\n", EVENT_CONTEXT_TYPE.trim()),
            "HostEventContext & ",
        )
    } else {
        (String::new(), "")
    };
    let ctx_type = format!("{events}{{ readonly hot: HostHotContext }}");
    format!(
        "{event_context}{}\n\ndeclare const ctx: {ctx_type};\n\n{}\n",
        HOT_CONTEXT_TYPE.trim(),
        ENV_MODULE_TYPE.trim().replace("__CTX__", &ctx_type)
    )
}

#[test]
fn sdk_generates_function_wrapper_from_contract_model() {
    let sdk = render_typescript_sdk(&[find_user()]);

    assert!(sdk.contains("type FindUserInput = number;"));
    assert!(sdk.contains("type FindUserOutput = string;"));
    assert!(sdk.contains("function __hostCall<T>(name: string, input?: unknown): T"));
    assert!(sdk.contains("export const user = {"));
    assert!(sdk.contains("find(input: FindUserInput): FindUserOutput"));
    assert!(sdk.contains("return __hostCall<FindUserOutput>(\"user.find\", input);"));
    assert!(sdk.contains("export const manivelleSdk = {"));
    assert!(sdk.contains("functions: {"));
    assert!(sdk.contains("user,"));
}

#[test]
fn sdk_generates_event_wrapper_from_callback_contracts() {
    let sdk = render_typescript_sdk(&[demo_callback()]);

    assert!(sdk.contains("type DemoCallbackPayload = { combo: number; };"));
    assert!(sdk.contains("type HostEvents = {"));
    assert!(sdk.contains("\"demo.callback\": DemoCallbackPayload;"));
    assert!(sdk.contains("export const ctx = __ctx;"));
    assert!(sdk.contains("export const events = {"));
    assert!(sdk.contains("callback(handler: HostEventHandler<\"demo.callback\">): void"));
    assert!(sdk.contains("return __ctx.on(\"demo.callback\", handler);"));
    assert!(sdk.contains("export const manivelleSdk = {"));
    assert!(sdk.contains("events,"));
    assert!(sdk.contains("ctx,"));
}

mod lock_tests {
    use std::sync::Arc;

    use rquickjs::{Context, Function, Object, Runtime};

    use super::super::bindings::{FunctionBindingStore, NativeContext};
    use crate::contract::{HostContract, HostContractKind, HostFunctionSignature};
    use crate::runner::host_fn::HostFnClass;
    use crate::runner::host_promises::HostPromises;

    struct Reentrant;

    impl HostContract for Reentrant {
        const NAME: &'static str = "test.reentrant";

        fn kind() -> HostContractKind {
            HostContractKind::Function
        }
    }

    impl HostFunctionSignature for Reentrant {
        type Input = ();
        type Output = ();
    }

    #[test]
    fn handler_runs_without_registry_lock() {
        let store = Arc::new(FunctionBindingStore::default());
        let weak = Arc::downgrade(&store);
        store
            .insert_plain::<Reentrant>(move |()| {
                let store = weak.upgrade().expect("store alive");
                let registry = store.by_name.try_lock();
                assert!(
                    registry.is_ok(),
                    "handler must be able to register another binding"
                );
                Ok(())
            })
            .unwrap();
        let runtime = Runtime::new().unwrap();
        let context = Context::full(&runtime).unwrap();

        context.with(|ctx| {
            let target = Object::new(ctx.clone()).unwrap();
            store
                .install_native(
                    &target,
                    "script",
                    &HostPromises::default().downgrade(),
                    &NativeContext {
                        execution: Arc::default(),
                        class: HostFnClass::register(&ctx).unwrap(),
                    },
                    None,
                    |_| None,
                )
                .unwrap();
            let function: Function<'_> = target.get(Reentrant::NAME).unwrap();
            function.call::<_, ()>(()).unwrap();
        });
    }
}
