use manivelle::TsSchema;

#[derive(TsSchema)]
#[serde(transparent)]
struct InvalidTransparent {
    id: u64,
    name: String,
}

fn main() {}
