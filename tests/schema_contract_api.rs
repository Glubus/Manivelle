#![cfg(feature = "derive")]

use std::collections::HashMap;

use manivelle::{
    EngineError, HostContract, HostContractKind, HostFunction, HostFunctionSignature,
    InMemoryHostContractRegistry, TsSchema,
};

struct FindUsers;

#[derive(Debug, TsSchema)]
struct FindUsersInput {
    ids: Vec<u64>,
}

#[derive(Debug, TsSchema)]
struct FindUsersOutput {
    users: HashMap<String, Option<String>>,
}

impl HostContract for FindUsers {
    const NAME: &'static str = "user.findMany";
    const IMPORT_MODULE: &'static str = "test";
    const EXPORT_PATH: &'static [&'static str] = &["user", "findMany"];

    fn kind() -> HostContractKind {
        HostContractKind::Function
    }
}

impl HostFunctionSignature for FindUsers {
    type Input = FindUsersInput;
    type Output = FindUsersOutput;
}

impl HostFunction for FindUsers {
    fn call(input: Self::Input) -> Result<Self::Output, EngineError> {
        let users = input
            .ids
            .into_iter()
            .map(|id| (id.to_string(), Some(format!("user-{id}"))))
            .collect();
        Ok(FindUsersOutput { users })
    }
}

#[test]
fn host_contracts_can_emit_dts_from_rust_type_schema_trait() {
    let registry = InMemoryHostContractRegistry::new();
    registry.function::<FindUsers>().expect("register function");

    let dts = registry.dts().expect("render declarations");

    assert!(dts.contains("type FindUsersInput = { ids: number[]; };"));
    assert!(dts.contains("type FindUsersOutput = { users: Record<string, string | null>; };"));
    assert!(dts.contains("export function findMany(input: FindUsersInput): FindUsersOutput;"));
}
