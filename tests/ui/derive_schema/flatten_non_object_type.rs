use manivelle::TsSchema;

#[derive(TsSchema)]
struct InvalidFlatten {
    id: u32,
    #[serde(flatten)]
    labels: Option<Vec<String>>,
}

fn main() {}
