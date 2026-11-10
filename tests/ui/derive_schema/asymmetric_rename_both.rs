use manivelle::TsSchema;

#[derive(TsSchema)]
struct Directional {
    #[serde(rename(serialize = "outName", deserialize = "inName"))]
    value: u8,
}

fn main() {}
