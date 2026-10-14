use manivelle::TsSchema;
use serde::{Serialize, Serializer};

fn as_text<S: Serializer>(value: &u32, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&value.to_string())
}

#[derive(Serialize, TsSchema)]
#[manivelle(encode_only)]
struct Counter {
    #[serde(serialize_with = "as_text")]
    value: u32,
}

fn main() {}
