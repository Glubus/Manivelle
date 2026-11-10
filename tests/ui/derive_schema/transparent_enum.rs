use manivelle::TsSchema;

#[derive(TsSchema)]
#[serde(transparent)]
enum Shape {
    Circle(f64),
}

fn main() {}
