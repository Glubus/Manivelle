use manivelle::TsSchema;

#[derive(TsSchema)]
#[manivelle(encode_only, decode_only)]
struct Conflict {
    value: String,
}

fn main() {}
