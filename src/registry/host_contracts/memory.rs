use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::path::Path;
use std::sync::{Arc, Mutex, MutexGuard, OnceLock};

use rquickjs::{Object, Result as JsResult, Value as JsValue};

use super::api_schema::render_api_schema;
use super::bindings::{
    ContractValidator, FunctionBindingStore, NativeContext, Refusal, duplicate_contract,
};
use super::declarations::render_typescript_declarations;
use super::import_modules::{HostModuleStyle, render_host_import_modules};
use super::names::is_binding_name;
use super::sdk::render_typescript_sdk;
use crate::config::{ContractValidation, UnknownFieldValidation};
use crate::contract::JsView;
use crate::contract::validation::{Invalid, SchemaValidationOptions, validate_schema_with_options};
use crate::contract::{
    Caller, HostCallback, HostCallbackDescriptor, HostContext, HostContractAbi,
    HostContractDescriptor, HostContractKind, HostFunctionDescriptor, HostFunctionSignature,
    HostImportBinding, HostMetadata, HostRequest, HostResolver, JsDecode, JsEncode,
    RegisterHostFunction, Schema, ScriptExport, TsArgs, TsSchema,
};
use crate::error::{Direction, EngineError};
use crate::packages::PermissionSet;
use crate::runner::host_promises::HostPromises;
use crate::sdk_files::{
    GeneratedSdkFiles, SdkFileNames, write_host_sdk_files, write_host_sdk_files_with_names,
};

/// In-memory host contract registry.
#[derive(Default)]
pub struct InMemoryHostContractRegistry {
    by_name: Mutex<HashMap<String, HostContractDescriptor>>,
    function_bindings: FunctionBindingStore,
    validation: ContractValidation,
    unknown_field_validation: UnknownFieldValidation,
}

impl InMemoryHostContractRegistry {
    /// Creates an empty host contract registry.
    pub fn new() -> Self {
        Self::default()
    }

    /// Creates an empty host contract registry with a validation policy.
    pub fn with_validation(validation: ContractValidation) -> Self {
        Self {
            validation,
            ..Self::default()
        }
    }

    /// Creates an empty host contract registry with explicit validation policies.
    pub fn with_validation_options(
        validation: ContractValidation,
        unknown_field_validation: UnknownFieldValidation,
    ) -> Self {
        Self {
            validation,
            unknown_field_validation,
            ..Self::default()
        }
    }

    /// Registers one host function with its handler, and returns the registry for
    /// chaining. This is the one way to register a function declared with `#[host_fn]`,
    /// or a `HostFunction` written by hand.
    ///
    /// The schemas come from `TsSchema` of the input and output types. Calls from
    /// scripts convert the input and output natively through [`JsDecode`] and
    /// [`JsEncode`].
    ///
    /// Every function registration refuses, with [`EngineError::InvalidOptions`], a name
    /// with a dot-separated segment that is a reserved word (`demo.function`) or not an
    /// identifier, because the generated TypeScript spells each segment as a namespace
    /// or function name. A name already taken is [`EngineError::DuplicateContract`].
    pub fn function<T>(&self) -> Result<&Self, EngineError>
    where
        T: RegisterHostFunction,
    {
        T::register_in(self)
    }

    /// Registers contract `C` implemented by `handler`, a closure that can hold state,
    /// and returns the registry for chaining.
    ///
    /// Like [`Self::function`], the schemas come from `TsSchema` of the input and
    /// output types, and values cross natively.
    pub fn function_with<C>(
        &self,
        handler: impl Fn(C::Input) -> Result<C::Output, EngineError> + Send + Sync + 'static,
    ) -> Result<&Self, EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode,
    {
        self.insert_descriptor(function_descriptor::<C>(false), |bindings| {
            bindings.insert_plain::<C>(handler)
        })?;
        Ok(self)
    }

    /// Registers contract `C` implemented by `handler`, which also receives the
    /// [`Caller`], and returns the registry for chaining.
    ///
    /// Like [`Self::function_with`] otherwise.
    pub fn function_with_caller<C>(
        &self,
        handler: impl Fn(&Caller<'_>, C::Input) -> Result<C::Output, EngineError>
        + Send
        + Sync
        + 'static,
    ) -> Result<&Self, EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode,
    {
        self.insert_descriptor(function_descriptor::<C>(false), |bindings| {
            bindings.insert_with_caller::<C>(handler)
        })?;
        Ok(self)
    }

    /// Registers async contract `C` implemented by `handler`, and returns the registry
    /// for chaining.
    ///
    /// Scripts receive a `Promise` of the output, which the [`HostResolver`] passed to
    /// `handler` settles. The schemas and the native conversion are those of
    /// [`Self::function_with`]; the output is converted to JavaScript on the engine
    /// thread, so it must be `Send`.
    pub fn async_function_with<C>(
        &self,
        handler: impl Fn(C::Input, HostResolver<C::Output>) -> Result<(), EngineError>
        + Send
        + Sync
        + 'static,
    ) -> Result<&Self, EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode + Send,
    {
        self.insert_descriptor(function_descriptor::<C>(true), |bindings| {
            bindings.insert_async::<C, _>(move |_, input, resolver| handler(input, resolver))
        })?;
        Ok(self)
    }

    /// Registers async contract `C` implemented by `handler`, which also receives the
    /// [`Caller`], and returns the registry for chaining.
    ///
    /// Like [`Self::async_function_with`] otherwise.
    pub fn async_function_with_caller<C>(
        &self,
        handler: impl Fn(&Caller<'_>, C::Input, HostResolver<C::Output>) -> Result<(), EngineError>
        + Send
        + Sync
        + 'static,
    ) -> Result<&Self, EngineError>
    where
        C: HostFunctionSignature + 'static,
        C::Input: TsSchema + JsDecode,
        C::Output: TsSchema + JsEncode + Send,
    {
        self.insert_descriptor(function_descriptor::<C>(true), |bindings| {
            bindings.insert_async::<C, _>(handler)
        })?;
        Ok(self)
    }

    /// Registers one host callback using `TsSchema` from its payload type and returns the
    /// registry for chaining.
    pub fn callback<T>(&self) -> Result<&Self, EngineError>
    where
        T: HostCallback + 'static,
        T::Payload: TsSchema + JsEncode,
    {
        self.insert_descriptor(callback_descriptor::<T>(None), |_| Ok(()))?;
        Ok(self)
    }

    /// Registers one host request, a callback whose handlers reply, using `TsSchema` from
    /// its payload and reply types, and returns the registry for chaining.
    pub fn request<T>(&self) -> Result<&Self, EngineError>
    where
        T: HostRequest + 'static,
        T::Payload: TsSchema + JsEncode,
        T::Reply: TsSchema + JsDecode,
    {
        self.insert_descriptor(callback_descriptor::<T>(Some(T::Reply::schema())), |_| {
            Ok(())
        })?;
        Ok(self)
    }

    /// Registers one host context contract and returns the registry for chaining.
    pub fn context<T>(&self) -> Result<&Self, EngineError>
    where
        T: HostContext,
    {
        let schema = T::schema();
        let descriptor = described(
            T::descriptor(),
            schema.clone(),
            HostContractAbi::Context { schema },
        );
        self.insert_descriptor(descriptor, |_| Ok(()))?;
        Ok(self)
    }

    /// Registers a function the scripts export, so that the generated TypeScript holds
    /// its signature in `ScriptExports` and a script can annotate its export with it for
    /// `tsc` to check. The schemas come from `TsSchema` of the argument types and of the
    /// output type. Returns the registry for chaining.
    ///
    /// This only describes the export: it is not a host contract, scripts cannot call
    /// it, and it does not change how [`Engine::call_typed`](crate::Engine::call_typed)
    /// calls it.
    pub fn script_export<E>(&self) -> Result<&Self, EngineError>
    where
        E: ScriptExport,
        E::Args: TsArgs,
        E::Output: TsSchema,
    {
        let output = E::Output::schema();
        let descriptor = HostContractDescriptor {
            name: E::NAME.to_owned(),
            kind: HostContractKind::Export,
            schema: output.clone(),
            metadata: HostMetadata {
                name: E::NAME.to_owned(),
                tags: Vec::new(),
            },
            import: HostImportBinding {
                module: String::new(),
                export_path: Vec::new(),
            },
            callback: None,
            function: None,
            abi: HostContractAbi::Export {
                args: E::Args::ts_schemas(),
                output,
            },
            docs: None,
        };
        // Under a key of its own, so that an export named like a host contract replaces
        // neither. Declaring the same export again replaces its description.
        let mut guard = self.lock_descriptors()?;
        refuse_conflicting_schemas(&descriptor, guard.values())?;
        guard.insert(format!("export {}", E::NAME), descriptor);
        Ok(self)
    }

    /// Returns one descriptor by stable contract name.
    pub fn get(&self, name: &str) -> Result<Option<HostContractDescriptor>, EngineError> {
        let guard = self.lock_descriptors()?;
        Ok(guard.get(name).cloned())
    }

    /// Returns every descriptor known by the registry, sorted by name.
    pub fn descriptors(&self) -> Result<Vec<HostContractDescriptor>, EngineError> {
        let guard = self.lock_descriptors()?;
        let mut descriptors = guard.values().cloned().collect::<Vec<_>>();
        descriptors.sort_by(|left, right| left.name.cmp(&right.name));
        Ok(descriptors)
    }

    pub(crate) fn import_module_names(&self) -> Result<BTreeSet<String>, EngineError> {
        Ok(self
            .lock_descriptors()?
            .values()
            .map(|descriptor| descriptor.import.module.clone())
            .collect())
    }

    /// Source of every host import module, keyed by module name, as the script of a
    /// context of its own or of a context group imports it.
    pub(crate) fn import_modules(
        &self,
        style: HostModuleStyle,
    ) -> Result<BTreeMap<String, String>, EngineError> {
        Ok(render_host_import_modules(&self.descriptors()?, style))
    }

    /// Installs every host function of script `script_id` on `target` as a native
    /// QuickJS function keyed by contract name. Async functions start their calls in
    /// `promises`, which they hold weakly.
    pub(crate) fn install_native_functions<'js>(
        self: &Arc<Self>,
        target: &Object<'js>,
        script_id: &str,
        promises: &HostPromises,
        native: &NativeContext,
        allowed: Option<&PermissionSet>,
    ) -> JsResult<()> {
        let promises = promises.downgrade();
        self.function_bindings.install_native(
            target,
            script_id,
            &promises,
            native,
            allowed,
            |name| self.validates_any().then(|| self.contract_validator(name)),
        )
    }

    fn validates_any(&self) -> bool {
        self.validation.validates_inputs() || self.validation.validates_outputs()
    }

    /// Validates the values of contract `name` against its registered descriptor.
    fn contract_validator(self: &Arc<Self>, name: &str) -> Arc<dyn ContractValidator> {
        Arc::new(RegisteredContract {
            registry: Arc::clone(self),
            name: name.to_owned(),
            schemas: OnceLock::new(),
        })
    }

    /// Renders TypeScript declarations from registered host contracts.
    pub fn dts(&self) -> Result<String, EngineError> {
        Ok(render_typescript_declarations(&self.descriptors()?))
    }

    /// Renders an ergonomic TypeScript SDK from registered host contracts.
    pub fn sdk(&self) -> Result<String, EngineError> {
        Ok(render_typescript_sdk(&self.descriptors()?))
    }

    /// Describes every registered contract in one JSON document, in the manner of
    /// OpenAPI: `functions`, `events`, script `exports` and `contexts` by name, each with
    /// its `description` (the `///` comment), the `module` and `path` scripts import it
    /// from when it has one, and its values as JSON Schema 2020-12; named types are under
    /// `schemas` and referenced with `$ref`. A function that takes or returns nothing
    /// has no `input` or `output`, and one that returns a `Promise` has `"async": true`.
    /// `manivelle` is the version of this layout.
    pub fn api_schema(&self) -> Result<serde_json::Value, EngineError> {
        Ok(render_api_schema(&self.descriptors()?))
    }

    /// Writes generated declaration and SDK source files into one directory.
    pub fn write_sdk_files(
        &self,
        directory: impl AsRef<Path>,
    ) -> Result<GeneratedSdkFiles, EngineError> {
        write_host_sdk_files(directory, &self.descriptors()?)
    }

    /// Writes generated declaration and SDK source files with explicit file names.
    pub fn write_sdk_files_with_names(
        &self,
        directory: impl AsRef<Path>,
        names: &SdkFileNames,
    ) -> Result<GeneratedSdkFiles, EngineError> {
        write_host_sdk_files_with_names(directory, &self.descriptors()?, names)
    }

    /// Locks the descriptors. Each update holds the lock across its check and its insert.
    fn lock_descriptors(
        &self,
    ) -> Result<MutexGuard<'_, HashMap<String, HostContractDescriptor>>, EngineError> {
        self.by_name.lock().map_err(|_| EngineError::LockPoisoned)
    }

    /// Registers `descriptor` under its name, and `bind` installs the function it describes.
    /// Both happen under one lock, so a name already taken fails with nothing changed.
    /// A function whose name has a segment that is a reserved word, or no identifier, is
    /// refused: the declarations spell each segment as a namespace or function name.
    fn insert_descriptor(
        &self,
        descriptor: HostContractDescriptor,
        bind: impl FnOnce(&FunctionBindingStore) -> Result<(), EngineError>,
    ) -> Result<(), EngineError> {
        if descriptor.function.is_some() && !descriptor.name.split('.').all(is_binding_name) {
            return Err(EngineError::InvalidOptions {
                details: format!(
                    "host function contract `{}` has a name segment that is a reserved word or not an identifier, so no TypeScript declaration can spell it",
                    descriptor.name
                ),
            });
        }
        let mut guard = self.lock_descriptors()?;
        if guard.contains_key(&descriptor.name) {
            return Err(duplicate_contract(&descriptor.name));
        }
        refuse_conflicting_schemas(&descriptor, guard.values())?;
        bind(&self.function_bindings)?;
        guard.insert(descriptor.name.clone(), descriptor);
        Ok(())
    }

    /// Checks `value` against `schema`, one side of contract `name`.
    fn check(
        &self,
        name: &str,
        direction: Direction,
        schema: &Schema,
        value: &JsValue<'_>,
    ) -> Result<(), Refusal> {
        validate_schema_with_options(
            schema,
            &JsView::of(value),
            validation_options(self.unknown_field_validation),
        )
        .map_err(|invalid| match invalid {
            Invalid::Mismatch(details) => Refusal::Contract(EngineError::ContractValidation {
                contract_name: name.to_owned(),
                direction,
                details,
            }),
            Invalid::Read(error) => Refusal::Unreadable(error),
        })
    }
}

fn validation_options(unknown_field_validation: UnknownFieldValidation) -> SchemaValidationOptions {
    SchemaValidationOptions {
        reject_unknown_fields: unknown_field_validation.rejects_unknown_fields(),
    }
}

/// Refuses `descriptor` when two of its named schemas, or one of them and one of a
/// `registered` descriptor, share a name but not a shape: a type reference resolves by
/// name, and the declarations hold one `type` per name.
fn refuse_conflicting_schemas<'a>(
    descriptor: &HostContractDescriptor,
    registered: impl Iterator<Item = &'a HostContractDescriptor>,
) -> Result<(), EngineError> {
    let mut new = Vec::new();
    descriptor_schemas(descriptor, &mut new);
    let clash = |name: &str, other: &str| EngineError::InvalidOptions {
        details: format!(
            "host contracts `{other}` and `{}` declare different types both named `{name}`",
            descriptor.name
        ),
    };
    for (index, schema) in new.iter().enumerate() {
        let Some(name) = schema.declared_name() else {
            continue;
        };
        if new[..index]
            .iter()
            .any(|earlier| earlier.declared_name() == Some(name) && earlier != schema)
        {
            return Err(clash(name, &descriptor.name));
        }
    }
    for other in registered {
        let mut known = Vec::new();
        descriptor_schemas(other, &mut known);
        for schema in &new {
            let Some(name) = schema.declared_name() else {
                continue;
            };
            if known
                .iter()
                .any(|k| k.declared_name() == Some(name) && k != schema)
            {
                return Err(clash(name, &other.name));
            }
        }
    }
    Ok(())
}

/// Every named schema `descriptor` uses, dependencies included.
fn descriptor_schemas<'a>(descriptor: &'a HostContractDescriptor, out: &mut Vec<&'a Schema>) {
    let roots: Vec<&Schema> = match &descriptor.abi {
        HostContractAbi::Function { input, output, .. } => vec![input, output],
        HostContractAbi::Callback { payload, reply } => {
            std::iter::once(payload).chain(reply.as_ref()).collect()
        }
        HostContractAbi::Context { schema } => vec![schema],
        HostContractAbi::Export { args, output } => args.iter().chain([output]).collect(),
        HostContractAbi::Unknown => Vec::new(),
    };
    for root in roots {
        collect_named(root, out);
    }
}

fn collect_named<'a>(schema: &'a Schema, out: &mut Vec<&'a Schema>) {
    if schema.declared_name().is_some() {
        out.push(schema);
    }
    for dependency in &schema.dependencies {
        collect_named(dependency, out);
    }
}

/// Validates one contract's values against the schemas of the descriptor registered under
/// its name.
struct RegisteredContract {
    registry: Arc<InMemoryHostContractRegistry>,
    name: String,
    /// The contract's schemas, copied out of the registry the first time a value is
    /// checked. A descriptor never changes once registered, and reading a value can run
    /// script code (a getter, a proxy) that calls back into the registry: no check may
    /// run under its lock.
    schemas: OnceLock<FunctionSchemas>,
}

struct FunctionSchemas {
    input: Schema,
    output: Schema,
}

impl RegisteredContract {
    /// The schemas of the contract, or `None` while it is not a registered function.
    fn schemas(&self) -> Result<Option<&FunctionSchemas>, EngineError> {
        if let Some(schemas) = self.schemas.get() {
            return Ok(Some(schemas));
        }
        let found = {
            let descriptors = self.registry.lock_descriptors()?;
            let function = descriptors
                .get(&self.name)
                .and_then(|descriptor| descriptor.function.as_ref());
            function.map(|function| FunctionSchemas {
                input: function.input_schema.clone(),
                output: function.output_schema.clone(),
            })
        };
        Ok(found.map(|found| self.schemas.get_or_init(|| found)))
    }
}

impl ContractValidator for RegisteredContract {
    fn checks_input(&self) -> bool {
        self.registry.validation.validates_inputs()
    }

    fn checks_output(&self) -> bool {
        self.registry.validation.validates_outputs()
    }

    fn validate_input(&self, input: &JsValue<'_>) -> Result<(), Refusal> {
        match self.schemas()? {
            Some(schemas) => {
                self.registry
                    .check(&self.name, Direction::Input, &schemas.input, input)
            }
            None => Ok(()),
        }
    }

    fn validate_output(&self, output: &JsValue<'_>) -> Result<(), Refusal> {
        match self.schemas()? {
            Some(schemas) => {
                self.registry
                    .check(&self.name, Direction::Output, &schemas.output, output)
            }
            None => Ok(()),
        }
    }
}

/// Descriptor of function contract `C` with the schemas `TsSchema` gives its input and
/// output types, returning a `Promise` of its output when `returns_promise` is set.
fn function_descriptor<C>(returns_promise: bool) -> HostContractDescriptor
where
    C: HostFunctionSignature,
    C::Input: TsSchema,
    C::Output: TsSchema,
{
    let function = HostFunctionDescriptor {
        input_schema: C::Input::schema(),
        output_schema: C::Output::schema(),
        returns_promise,
    };
    let abi = HostContractAbi::Function {
        input: function.input_schema.clone(),
        output: function.output_schema.clone(),
        returns_promise,
    };
    let mut descriptor = described(C::descriptor(), function.input_schema.clone(), abi);
    descriptor.function = Some(function);
    descriptor
}

/// Descriptor of callback contract `T` with the payload schema `TsSchema` gives its
/// payload type and, for a request, the schema of its handlers' `reply`.
fn callback_descriptor<T>(reply: Option<Schema>) -> HostContractDescriptor
where
    T: HostCallback,
    T::Payload: TsSchema,
{
    let callback = HostCallbackDescriptor {
        payload_schema: T::Payload::schema(),
        reply_schema: reply,
    };
    let abi = HostContractAbi::Callback {
        payload: callback.payload_schema.clone(),
        reply: callback.reply_schema.clone(),
    };
    let mut descriptor = described(T::descriptor(), callback.payload_schema.clone(), abi);
    descriptor.callback = Some(callback);
    descriptor
}

/// `descriptor` with the schema and the ABI that every kind of contract has.
fn described(
    mut descriptor: HostContractDescriptor,
    schema: Schema,
    abi: HostContractAbi,
) -> HostContractDescriptor {
    descriptor.schema = schema;
    descriptor.abi = abi;
    descriptor
}
