use manivelle::TsSchema;

#[derive(TsSchema)]
#[serde(untagged, tag = "kind")]
enum Shape {
    Circle { radius: f64 },
    Square { side: f64 },
}

fn main() {}
