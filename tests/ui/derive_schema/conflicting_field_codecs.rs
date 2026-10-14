use manivelle::TsSchema;

#[derive(TsSchema)]
struct Conflict {
    #[manivelle(codec = "json", with = "custom")]
    value: String,
}

fn main() {}
