use manivelle::{EngineError, host_fn};

#[host_fn(name = "math.add")]
fn add(left: f64, right: f64) -> Result<f64, EngineError> {
    Ok(left + right)
}

fn main() {}
