use std::collections::HashMap;
use std::hint::black_box;
use std::sync::{Arc, OnceLock};

use criterion::{BenchmarkId, Criterion, criterion_group, criterion_main};
use manivelle::{
    ContractValidation, Engine, EngineError, EngineOptions, HostContract, HostContractKind,
    HostFunction, HostFunctionSignature, JsEncode, NativeBytes, TsSchema,
};
use rquickjs::{
    ArrayBuffer, Context, Ctx, Function, Object, Result as JsResult, Runtime, TypedArray,
    prelude::Func,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};

const LARGE_FIELD_COUNT: usize = 64;
const BYTE_COUNTS: &[usize] = &[1024, 64 * 1024, 1024 * 1024];
/// Entry counts of the maps and JSON objects `Engine::call` encodes as arguments.
const OBJECT_ENTRY_COUNTS: &[usize] = &[16, 1024];
static FIXTURES: OnceLock<Arc<NativeFixtures>> = OnceLock::new();

fn native_bridge_benchmarks(c: &mut Criterion) {
    let bench = NativeBridgeBench::new();
    let integrated_disabled = IntegratedBridgeBench::new(ContractValidation::Disabled);
    let integrated_validated = IntegratedBridgeBench::new(ContractValidation::InputsAndOutputs);

    bench_struct_bridge_shapes(c, &bench);
    bench_byte_bridge_shapes(c, &bench);
    bench_integrated_bridge_shapes(c, &integrated_disabled, "validation_disabled");
    bench_integrated_bridge_shapes(c, &integrated_validated, "validation_inputs_outputs");
    bench_object_encoding(c);
}

fn bench_struct_bridge_shapes(c: &mut Criterion, bench: &NativeBridgeBench) {
    let mut group = c.benchmark_group("native_bridge_structs");
    group.sample_size(30);

    bench_js_function(&mut group, bench, "copy_small_struct", "sumCopySmall");
    bench_js_function(&mut group, bench, "getters_small_struct", "sumGetterSmall");
    bench_js_function(
        &mut group,
        bench,
        "copy_large_struct_all_fields",
        "sumCopyLarge",
    );
    bench_js_function(
        &mut group,
        bench,
        "getters_large_struct_all_fields",
        "sumGetterLargeAll",
    );
    bench_js_function(
        &mut group,
        bench,
        "getters_large_struct_one_field",
        "sumGetterLargeOne",
    );
    bench_js_function(
        &mut group,
        bench,
        "native_memory_large_all_fields",
        "sumNativeMemoryAll",
    );
    bench_js_function(
        &mut group,
        bench,
        "native_memory_large_one_field",
        "sumNativeMemoryOne",
    );

    group.finish();
}

fn bench_byte_bridge_shapes(c: &mut Criterion, bench: &NativeBridgeBench) {
    let mut group = c.benchmark_group("native_bridge_bytes");
    group.sample_size(20);

    for &byte_count in BYTE_COUNTS {
        bench_js_function_with_len(
            &mut group,
            bench,
            "json_array_bytes",
            "sumBytesJsonArray",
            byte_count,
        );
        bench_js_function_with_len(
            &mut group,
            bench,
            "uint8array_copy_bytes",
            "sumBytesCopiedArray",
            byte_count,
        );
        bench_js_function_with_len(
            &mut group,
            bench,
            "uint8array_shared_native_bytes",
            "sumBytesSharedArray",
            byte_count,
        );
    }

    group.finish();
}

fn bench_integrated_bridge_shapes(c: &mut Criterion, bench: &IntegratedBridgeBench, label: &str) {
    let mut group = c.benchmark_group(format!("native_bridge_integrated_{label}"));
    group.sample_size(10);

    bench_vm_function(
        &mut group,
        bench,
        "host_value_small_struct",
        "hostValueSmall",
    );
    bench_vm_function(
        &mut group,
        bench,
        "typed_host_value_small_struct",
        "typedHostValueSmall",
    );
    bench_vm_function(
        &mut group,
        bench,
        "host_value_large_struct_all_fields",
        "hostValueLargeAll",
    );
    bench_vm_function(
        &mut group,
        bench,
        "typed_host_value_large_struct_all_fields",
        "typedHostValueLargeAll",
    );
    bench_vm_function(
        &mut group,
        bench,
        "host_value_large_struct_one_field",
        "hostValueLargeOne",
    );
    bench_vm_function(
        &mut group,
        bench,
        "getter_large_all_fields",
        "getterLargeAll",
    );
    bench_vm_function(
        &mut group,
        bench,
        "getter_large_one_field",
        "getterLargeOne",
    );
    bench_vm_function(
        &mut group,
        bench,
        "native_memory_large_all_fields",
        "nativeMemoryLargeAll",
    );
    bench_vm_function(
        &mut group,
        bench,
        "native_memory_large_one_field",
        "nativeMemoryLargeOne",
    );

    for &byte_count in BYTE_COUNTS {
        bench_vm_function_with_len(
            &mut group,
            bench,
            "host_value_json_array_bytes",
            "hostValueJsonArrayBytes",
            byte_count,
        );
        bench_vm_function_with_len(
            &mut group,
            bench,
            "host_value_uint8array_from_json_array_bytes",
            "hostValueUint8ArrayFromJsonArrayBytes",
            byte_count,
        );
        bench_vm_function_with_len(
            &mut group,
            bench,
            "host_value_native_uint8array_bytes",
            "hostValueNativeUint8ArrayBytes",
            byte_count,
        );
    }

    group.finish();
}

/// `Engine::call` with one object argument: a `HashMap` keyed by strings or integers,
/// or a `serde_json::Value` object. The export only reads one property, so the time is
/// the encoding of every entry as an own property of a fresh object.
fn bench_object_encoding(c: &mut Criterion) {
    let mut engine = Engine::new(&EngineOptions::default()).expect("create object engine");
    engine
        .load_script("objects", OBJECT_ENCODING_SCRIPT)
        .expect("load object encoding script");

    let mut group = c.benchmark_group("native_bridge_encode_objects");
    group.sample_size(30);
    for &entry_count in OBJECT_ENTRY_COUNTS {
        let string_map: HashMap<String, u32> = (0..entry_count)
            .map(|index| (format!("key-{index}"), index as u32))
            .collect();
        let integer_map: HashMap<u32, u32> = (0..entry_count)
            .map(|index| (index as u32, index as u32))
            .collect();
        let json_object = Value::Object(
            (0..entry_count)
                .map(|index| (format!("key-{index}"), json!(index)))
                .collect(),
        );

        group.bench_with_input(
            BenchmarkId::new("hashmap_string_keys", entry_count),
            &string_map,
            |b, map| b.iter(|| black_box(call_touch(&engine, map))),
        );
        group.bench_with_input(
            BenchmarkId::new("hashmap_integer_keys", entry_count),
            &integer_map,
            |b, map| b.iter(|| black_box(call_touch(&engine, map))),
        );
        group.bench_with_input(
            BenchmarkId::new("json_object", entry_count),
            &json_object,
            |b, object| b.iter(|| black_box(call_touch(&engine, object))),
        );
    }
    group.finish();
}

fn call_touch(engine: &Engine, object: &impl JsEncode) -> f64 {
    engine
        .call("objects", "touch", (object,))
        .expect("call object encoding export")
}

fn bench_js_function(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    bench: &NativeBridgeBench,
    label: &str,
    function_name: &'static str,
) {
    group.bench_function(label, |b| {
        b.iter(|| black_box(bench.call_u64(function_name)));
    });
}

fn bench_js_function_with_len(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    bench: &NativeBridgeBench,
    label: &str,
    function_name: &'static str,
    byte_count: usize,
) {
    group.bench_with_input(BenchmarkId::new(label, byte_count), &byte_count, |b, _| {
        b.iter(|| black_box(bench.call_u64_with_len(function_name, byte_count)));
    });
}

fn bench_vm_function(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    bench: &IntegratedBridgeBench,
    label: &str,
    function_name: &'static str,
) {
    group.bench_function(label, |b| {
        b.iter(|| black_box(bench.call(function_name)));
    });
}

fn bench_vm_function_with_len(
    group: &mut criterion::BenchmarkGroup<'_, criterion::measurement::WallTime>,
    bench: &IntegratedBridgeBench,
    label: &str,
    function_name: &'static str,
    byte_count: usize,
) {
    group.bench_with_input(BenchmarkId::new(label, byte_count), &byte_count, |b, _| {
        b.iter(|| black_box(bench.call_with_len(function_name, byte_count)));
    });
}

struct NativeBridgeBench {
    _runtime: Runtime,
    context: Context,
}

impl NativeBridgeBench {
    fn new() -> Self {
        let runtime = Runtime::new().expect("create runtime");
        let context = Context::full(&runtime).expect("create context");
        let fixtures = Arc::new(NativeFixtures::new());
        let _ = FIXTURES.set(fixtures.clone());

        context.with(|ctx| install_bridge(ctx, fixtures).expect("install bridge"));

        Self {
            _runtime: runtime,
            context,
        }
    }

    fn call_u64(&self, function_name: &'static str) -> u64 {
        self.context.with(|ctx| {
            let function = ctx
                .globals()
                .get::<_, Function<'_>>(function_name)
                .expect("get benchmark function");
            function
                .call::<_, u64>(())
                .expect("call benchmark function")
        })
    }

    fn call_u64_with_len(&self, function_name: &'static str, byte_count: usize) -> u64 {
        self.context.with(|ctx| {
            let function = ctx
                .globals()
                .get::<_, Function<'_>>(function_name)
                .expect("get benchmark function");
            function
                .call::<_, u64>((byte_count,))
                .expect("call benchmark function")
        })
    }
}

struct IntegratedBridgeBench {
    engine: Engine,
}

impl IntegratedBridgeBench {
    fn new(validation: ContractValidation) -> Self {
        let mut engine = Engine::new(&EngineOptions {
            memory_limit_bytes: 512 * 1024 * 1024,
            contract_validation: validation,
            ..EngineOptions::default()
        })
        .expect("create integrated engine");

        let registry = engine.registry();
        registry
            .function::<HostSmallCopyJson>()
            .expect("register small json copy");
        registry
            .function::<HostLargeCopyJson>()
            .expect("register large json copy");
        registry
            .function::<HostSmallCopyTyped>()
            .expect("register small typed copy");
        registry
            .function::<HostLargeCopyTyped>()
            .expect("register large typed copy");
        registry
            .function::<HostGetField>()
            .expect("register getter");
        registry
            .function::<HostReadNativeU32>()
            .expect("register native memory read");
        registry
            .function::<HostBytesJsonArray>()
            .expect("register json array bytes");
        registry
            .function::<HostBytesNative>()
            .expect("register native bytes");

        engine
            .load_script("native-bridge", INTEGRATED_BRIDGE_SCRIPT)
            .expect("load integrated bridge script");

        Self { engine }
    }

    fn call(&self, function_name: &'static str) -> f64 {
        self.engine
            .call("native-bridge", function_name, ())
            .expect("call integrated benchmark export")
    }

    fn call_with_len(&self, function_name: &'static str, byte_count: usize) -> f64 {
        self.engine
            .call("native-bridge", function_name, (byte_count,))
            .expect("call integrated benchmark export")
    }
}

struct NativeFixtures {
    small: SmallFixture,
    large_fields: Arc<[u32]>,
    bytes_1k: Arc<[u8]>,
    bytes_64k: Arc<[u8]>,
    bytes_1m: Arc<[u8]>,
}

impl NativeFixtures {
    fn new() -> Self {
        Self {
            small: SmallFixture {
                id: 42,
                hp: 120,
                mp: 35,
            },
            large_fields: make_large_fields(),
            bytes_1k: make_bytes(1024),
            bytes_64k: make_bytes(64 * 1024),
            bytes_1m: make_bytes(1024 * 1024),
        }
    }

    fn bytes(&self, byte_count: usize) -> Arc<[u8]> {
        match byte_count {
            1024 => self.bytes_1k.clone(),
            65_536 => self.bytes_64k.clone(),
            1_048_576 => self.bytes_1m.clone(),
            _ => panic!("unsupported byte count: {byte_count}"),
        }
    }
}

#[derive(Clone, Copy)]
struct SmallFixture {
    id: u32,
    hp: u32,
    mp: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct EmptyInput {}

#[derive(Debug, Clone, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct SmallPayload {
    id: u32,
    hp: u32,
    mp: u32,
}

#[derive(Debug, Clone, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct LargePayload {
    fields: Vec<u32>,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct FieldReadInput {
    handle: u32,
    index: u32,
}

#[derive(Debug, Clone, Copy, Serialize, Deserialize, TsSchema)]
#[serde(rename_all = "camelCase")]
struct ByteInput {
    byte_count: usize,
}

/// Declares one bench host function: a unit struct with its contract, signature and body.
macro_rules! bench_host_function {
    ($ty:ident, $name:literal, $input:ty, $output:ty, |$arg:ident| $body:block) => {
        struct $ty;

        impl HostContract for $ty {
            const NAME: &'static str = $name;

            fn kind() -> HostContractKind {
                HostContractKind::Function
            }
        }

        impl HostFunctionSignature for $ty {
            type Input = $input;
            type Output = $output;
        }

        impl HostFunction for $ty {
            fn call($arg: Self::Input) -> Result<Self::Output, EngineError> $body
        }
    };
}

bench_host_function! {
    HostSmallCopyJson, "bench.small.copyJson", Value, Value, |_input| {
        let small = fixtures().small;
        Ok(json!({
            "id": small.id,
            "hp": small.hp,
            "mp": small.mp,
        }))
    }
}

bench_host_function! {
    HostLargeCopyJson, "bench.large.copyJson", Value, Value, |_input| {
        Ok(json!({ "fields": fixtures().large_fields.as_ref() }))
    }
}

bench_host_function! {
    HostSmallCopyTyped, "bench.small.copyTyped", EmptyInput, SmallPayload, |_input| {
        let small = fixtures().small;
        Ok(SmallPayload {
            id: small.id,
            hp: small.hp,
            mp: small.mp,
        })
    }
}

bench_host_function! {
    HostLargeCopyTyped, "bench.large.copyTyped", EmptyInput, LargePayload, |_input| {
        Ok(LargePayload {
            fields: fixtures().large_fields.to_vec(),
        })
    }
}

bench_host_function! {
    HostGetField, "bench.field.get", Value, u32, |input| {
        let input: FieldReadInput =
            serde_json::from_value(input).map_err(|error| EngineError::Execution {
                details: error.to_string(),
            })?;
        Ok(get_field(fixtures(), input.handle, input.index))
    }
}

bench_host_function! {
    HostReadNativeU32, "bench.native.readU32", Value, u32, |input| {
        let input: FieldReadInput =
            serde_json::from_value(input).map_err(|error| EngineError::Execution {
                details: error.to_string(),
            })?;
        Ok(read_native_u32(fixtures(), input.handle, input.index))
    }
}

bench_host_function! {
    HostBytesJsonArray, "bench.bytes.jsonArray", Value, Value, |input| {
        let input = parse_byte_input(input)?;
        Ok(json!(fixtures().bytes(input.byte_count).as_ref()))
    }
}

bench_host_function! {
    HostBytesNative, "bench.bytes.native", Value, NativeBytes, |input| {
        let input = parse_byte_input(input)?;
        Ok(NativeBytes::from_shared(fixtures().bytes(input.byte_count)))
    }
}

fn parse_byte_input(input: Value) -> Result<ByteInput, EngineError> {
    serde_json::from_value(input).map_err(|error| EngineError::Execution {
        details: error.to_string(),
    })
}

fn install_bridge(ctx: Ctx<'_>, _fixtures: Arc<NativeFixtures>) -> JsResult<()> {
    let globals = ctx.globals();

    globals.set("copySmall", Func::from(copy_small_bridge))?;
    globals.set("copyLarge", Func::from(copy_large_bridge))?;
    globals.set("getField", Func::from(get_field_bridge))?;
    globals.set("readNativeU32", Func::from(read_native_u32_bridge))?;
    globals.set("bytesJsonArray", Func::from(bytes_json_array_bridge))?;
    globals.set("bytesCopiedArray", Func::from(bytes_copied_array_bridge))?;
    globals.set("bytesSharedArray", Func::from(bytes_shared_array_bridge))?;

    ctx.eval::<(), _>(BRIDGE_SCRIPT)
}

fn fixtures() -> &'static NativeFixtures {
    FIXTURES.get().expect("fixtures installed").as_ref()
}

fn copy_small_bridge<'js>(ctx: Ctx<'js>) -> JsResult<Object<'js>> {
    copy_small_struct(ctx, &fixtures().small)
}

fn copy_large_bridge<'js>(ctx: Ctx<'js>) -> JsResult<Object<'js>> {
    copy_large_struct(ctx, fixtures().large_fields.as_ref())
}

fn get_field_bridge(handle: u32, index: u32) -> u32 {
    get_field(fixtures(), handle, index)
}

fn read_native_u32_bridge(handle: u32, offset: u32) -> u32 {
    read_native_u32(fixtures(), handle, offset)
}

fn bytes_json_array_bridge(byte_count: usize) -> Vec<u8> {
    fixtures().bytes(byte_count).to_vec()
}

fn bytes_copied_array_bridge<'js>(
    ctx: Ctx<'js>,
    byte_count: usize,
) -> JsResult<TypedArray<'js, u8>> {
    TypedArray::<u8>::new_copy(ctx, fixtures().bytes(byte_count).as_ref())
}

fn bytes_shared_array_bridge<'js>(
    ctx: Ctx<'js>,
    byte_count: usize,
) -> JsResult<TypedArray<'js, u8>> {
    let buffer = ArrayBuffer::from_source_immutable(ctx, fixtures().bytes(byte_count))?;
    TypedArray::<u8>::from_arraybuffer(buffer)
}

fn copy_small_struct<'js>(ctx: Ctx<'js>, small: &SmallFixture) -> JsResult<Object<'js>> {
    let object = Object::new(ctx)?;
    object.set("id", small.id)?;
    object.set("hp", small.hp)?;
    object.set("mp", small.mp)?;
    Ok(object)
}

fn copy_large_struct<'js>(ctx: Ctx<'js>, fields: &[u32]) -> JsResult<Object<'js>> {
    let object = Object::new(ctx)?;
    for (index, value) in fields.iter().copied().enumerate() {
        object.set(format!("f{index}"), value)?;
    }
    object.set("fieldCount", fields.len() as u32)?;
    Ok(object)
}

fn get_field(fixtures: &NativeFixtures, handle: u32, index: u32) -> u32 {
    match handle {
        1 => match index {
            0 => fixtures.small.id,
            1 => fixtures.small.hp,
            2 => fixtures.small.mp,
            _ => 0,
        },
        2 => fixtures
            .large_fields
            .get(index as usize)
            .copied()
            .unwrap_or_default(),
        _ => 0,
    }
}

fn read_native_u32(fixtures: &NativeFixtures, handle: u32, offset: u32) -> u32 {
    match handle {
        2 => fixtures
            .large_fields
            .get(offset as usize)
            .copied()
            .unwrap_or_default(),
        _ => 0,
    }
}

fn make_large_fields() -> Arc<[u32]> {
    (0..LARGE_FIELD_COUNT)
        .map(|index| (index as u32 + 1) * 3)
        .collect::<Vec<_>>()
        .into()
}

fn make_bytes(byte_count: usize) -> Arc<[u8]> {
    (0..byte_count)
        .map(|index| (index % 251) as u8)
        .collect::<Vec<_>>()
        .into()
}

const BRIDGE_SCRIPT: &str = r#"
globalThis.sumCopySmall = function () {
  const value = copySmall();
  return value.id + value.hp + value.mp;
};

globalThis.sumGetterSmall = function () {
  return getField(1, 0) + getField(1, 1) + getField(1, 2);
};

globalThis.sumCopyLarge = function () {
  const value = copyLarge();
  let total = 0;
  for (let index = 0; index < value.fieldCount; index += 1) {
    total += value["f" + index];
  }
  return total;
};

globalThis.sumGetterLargeAll = function () {
  let total = 0;
  for (let index = 0; index < 64; index += 1) {
    total += getField(2, index);
  }
  return total;
};

globalThis.sumGetterLargeOne = function () {
  return getField(2, 7);
};

globalThis.sumNativeMemoryAll = function () {
  let total = 0;
  for (let offset = 0; offset < 64; offset += 1) {
    total += readNativeU32(2, offset);
  }
  return total;
};

globalThis.sumNativeMemoryOne = function () {
  return readNativeU32(2, 7);
};

globalThis.sumBytesJsonArray = function (byteCount) {
  const bytes = bytesJsonArray(byteCount);
  let total = 0;
  for (let index = 0; index < bytes.length; index += 4096) {
    total += bytes[index];
  }
  return total + bytes.length;
};

globalThis.sumBytesCopiedArray = function (byteCount) {
  const bytes = bytesCopiedArray(byteCount);
  let total = 0;
  for (let index = 0; index < bytes.length; index += 4096) {
    total += bytes[index];
  }
  return total + bytes.length;
};

globalThis.sumBytesSharedArray = function (byteCount) {
  const bytes = bytesSharedArray(byteCount);
  let total = 0;
  for (let index = 0; index < bytes.length; index += 4096) {
    total += bytes[index];
  }
  return total + bytes.length;
};
"#;

const INTEGRATED_BRIDGE_SCRIPT: &str = r#"
function hostValue(name, input) {
  return globalThis.__host.callValue(name, input);
}

function sumFields(fields) {
  let total = 0;
  for (let index = 0; index < fields.length; index += 1) {
    total += fields[index];
  }
  return total;
}

function sumSparseBytes(bytes) {
  let total = 0;
  for (let index = 0; index < bytes.length; index += 4096) {
    total += bytes[index];
  }
  return total + bytes.length;
}

export function hostValueSmall() {
  const value = hostValue("bench.small.copyJson", null);
  return value.id + value.hp + value.mp;
}

export function typedHostValueSmall() {
  const value = hostValue("bench.small.copyTyped", {});
  return value.id + value.hp + value.mp;
}

export function hostValueLargeAll() {
  return sumFields(hostValue("bench.large.copyJson", null).fields);
}

export function typedHostValueLargeAll() {
  return sumFields(hostValue("bench.large.copyTyped", {}).fields);
}

export function hostValueLargeOne() {
  return hostValue("bench.large.copyJson", null).fields[7];
}

export function getterLargeAll() {
  let total = 0;
  for (let index = 0; index < 64; index += 1) {
    total += hostValue("bench.field.get", { handle: 2, index });
  }
  return total;
}

export function getterLargeOne() {
  return hostValue("bench.field.get", { handle: 2, index: 7 });
}

export function nativeMemoryLargeAll() {
  let total = 0;
  for (let index = 0; index < 64; index += 1) {
    total += hostValue("bench.native.readU32", { handle: 2, index });
  }
  return total;
}

export function nativeMemoryLargeOne() {
  return hostValue("bench.native.readU32", { handle: 2, index: 7 });
}

export function hostValueJsonArrayBytes(byteCount) {
  return sumSparseBytes(hostValue("bench.bytes.jsonArray", { byteCount }));
}

export function hostValueUint8ArrayFromJsonArrayBytes(byteCount) {
  return sumSparseBytes(new Uint8Array(hostValue("bench.bytes.jsonArray", { byteCount })));
}

export function hostValueNativeUint8ArrayBytes(byteCount) {
  return sumSparseBytes(hostValue("bench.bytes.native", { byteCount }));
}
"#;

const OBJECT_ENCODING_SCRIPT: &str = r#"
export function touch(object) {
  return object["key-1"] ?? object[1];
}
"#;

criterion_group!(benches, native_bridge_benchmarks);
criterion_main!(benches);
