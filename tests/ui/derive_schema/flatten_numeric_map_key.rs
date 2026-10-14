use std::collections::{BTreeMap, HashMap};

use manivelle::TsSchema;

#[derive(TsSchema)]
struct FlattenedNumericMap {
    id: u32,
    #[serde(flatten)]
    extra: HashMap<u32, String>,
}

#[derive(TsSchema)]
#[serde(tag = "kind")]
enum TaggedNumericMap {
    Scores(BTreeMap<i64, f64>),
}

#[derive(TsSchema)]
#[serde(tag = "kind")]
enum TaggedScalar {
    Count(u32),
}

fn main() {}
