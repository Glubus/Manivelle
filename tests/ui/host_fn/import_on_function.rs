use manivelle::{EngineError, host_fn};

#[host_fn(name = "math.add", import = "app")]
fn add(input: f64) -> Result<f64, EngineError> {
    Ok(input)
}

fn main() {}
