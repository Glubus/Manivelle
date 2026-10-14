use manivelle::TsSchema;

#[derive(TsSchema)]
#[serde(tag = "type")]
struct Tagged {
    value: u32,
}

fn main() {}
