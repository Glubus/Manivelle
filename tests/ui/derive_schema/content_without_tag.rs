use manivelle::TsSchema;

#[derive(TsSchema)]
#[serde(content = "data")]
enum Shape {
    Circle { radius: f64 },
    Square { side: f64 },
}

fn main() {}
