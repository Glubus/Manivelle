#![cfg(feature = "derive")]

mod support;

use manivelle::{
    EngineError, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
    InMemoryHostContractRegistry, TsSchema, TsType,
};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize, TsSchema)]
#[serde(transparent)]
struct UserId(u64);

#[derive(Debug, Serialize, TsSchema)]
#[serde(transparent)]
struct SessionToken {
    value: String,
}

struct FindSession;
struct AutoCreateInvoice;
struct AutoInvoiceCreated;

#[derive(Debug, Deserialize, TsSchema)]
struct FindSessionInput {
    user_id: UserId,
}

#[derive(Debug, Serialize, TsSchema)]
struct FindSessionOutput {
    token: SessionToken,
}

#[derive(Debug, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct AutoCreateInvoiceInput {
    account_id: String,
    total: f32,
}

#[derive(Debug, Serialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct AutoCreateInvoiceOutput {
    invoice_id: String,
    accepted: bool,
}

#[derive(Debug, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct AutoInvoiceCreatedPayload {
    invoice_id: String,
    total: f32,
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
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        Ok(input)
    }
}

struct CreateInvoice;

#[derive(Debug, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct CreateInvoiceInput {
    account_id: String,
    total: f32,
    memo: Option<String>,
    #[serde(default)]
    retry_count: u32,
}

#[derive(Debug, Serialize, TsSchema)]
#[manivelle(name = "InvoiceCreated")]
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
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
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
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
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
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
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

#[test]
fn derive_ts_schema_for_serde_transparent_newtypes() {
    assert_eq!(UserId::schema_name(), Some("UserId"));
    assert_eq!(UserId::ts_type(), TsType::Number);
    assert_eq!(SessionToken::schema_name(), Some("SessionToken"));
    assert_eq!(SessionToken::ts_type(), TsType::String);
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
        .get(AutoCreateInvoice::NAME)
        .expect("get descriptor")
        .expect("typed function descriptor");
    let function = descriptor.function.expect("function metadata");
    let dts = registry.dts().expect("render declarations");
    let sdk = registry.sdk().expect("render SDK");

    assert_eq!(
        descriptor.schema.name.as_deref(),
        Some("AutoCreateInvoiceInput")
    );
    assert_eq!(
        function.input_schema.name.as_deref(),
        Some("AutoCreateInvoiceInput")
    );
    assert_eq!(
        function.output_schema.name.as_deref(),
        Some("AutoCreateInvoiceOutput")
    );
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
        .get(AutoInvoiceCreated::NAME)
        .expect("get descriptor")
        .expect("typed callback descriptor");
    let callback = descriptor.callback.expect("callback metadata");
    let dts = registry.dts().expect("render declarations");
    let sdk = registry.sdk().expect("render SDK");

    assert_eq!(
        descriptor.schema.name.as_deref(),
        Some("AutoInvoiceCreatedPayload")
    );
    assert_eq!(
        callback.payload_schema.name.as_deref(),
        Some("AutoInvoiceCreatedPayload")
    );
    assert!(
        dts.contains("type AutoInvoiceCreatedPayload = { invoiceId: string; total: number; };")
    );
    assert!(dts.contains("\"billing.invoice.created\": AutoInvoiceCreatedPayload;"));
    assert!(sdk.contains("created(handler: HostEventHandler<\"billing.invoice.created\">): void"));
}
