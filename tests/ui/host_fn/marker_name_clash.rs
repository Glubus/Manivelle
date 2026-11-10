use manivelle::{EngineError, host_fn};

struct Add;

#[host_fn(name = "math.add")]
fn add(input: f64) -> Result<f64, EngineError> {
    Ok(input)
}

fn main() {}
