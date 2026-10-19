#![cfg(feature = "derive")]

#[test]
fn derive_ts_schema_rejects_unsupported_shapes_and_attrs() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/derive_schema/*.rs");
}

#[test]
fn host_fn_attr_rejects_invalid_signatures() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/host_fn/*.rs");
}
