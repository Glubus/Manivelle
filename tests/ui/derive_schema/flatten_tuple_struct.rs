use manivelle::TsSchema;

#[derive(TsSchema)]
struct Inner {
    value: String,
}

#[derive(TsSchema)]
struct InvalidFlatten(#[serde(flatten)] Inner);

fn main() {}
